//! Conflict resolver.
//!
//! Handles file version conflicts and implements Syncthing's conflict
//! resolution strategy.
//!
//! See "docs/design/ADR-20261105-conflict-arbitration.md".

use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{ConflictResolution, EventPublisher, SyncEvent};
use std::path::Path;
use std::sync::{Arc, RwLock};
use syncthing_core::types::{ConcurrentOrder, FileInfo, Vector};
use tokio::fs;
use tracing::{debug, info, warn};

/// Resolves version conflicts between local and remote file states.
pub struct ConflictResolver {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    /// Local device ID (used to name conflict files; injected by the service
    /// layer after construction).
    local_device_id: RwLock<Option<syncthing_core::DeviceId>>,
}

impl ConflictResolver {
    /// Creates a new conflict resolver.
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher) -> Self {
        Self {
            db,
            events,
            local_device_id: RwLock::new(None),
        }
    }

    /// Injects the local device ID (used as the origin tag in conflict file names).
    pub fn set_local_device_id(&self, id: syncthing_core::DeviceId) {
        if let Ok(mut guard) = self.local_device_id.write() {
            *guard = Some(id);
        }
    }

    /// Determines the deterministic winner of a conflict.
    ///
    /// Aligned with Go `FileInfo.WinsConflict` (bep_fileinfo.go). Returns
    /// `true` when the remote side wins: newer mtime wins; on mtime ties the
    /// version vector's concurrent direction (first divergent counter)
    /// decides. Both peers compute the same winner independently, so the
    /// conflict converges within one index exchange. See
    /// "docs/design/ADR-20261105-conflict-arbitration.md".
    pub fn remote_wins_conflict(&self, local: &FileInfo, remote: &FileInfo) -> bool {
        if (remote.modified_s, remote.modified_ns) > (local.modified_s, local.modified_ns) {
            return true;
        }
        if (remote.modified_s, remote.modified_ns) < (local.modified_s, local.modified_ns) {
            return false;
        }
        matches!(
            remote.version.concurrent_order(&local.version),
            Some(ConcurrentOrder::SelfGreater)
        )
    }

    /// Checks for and resolves a conflict between the local and remote versions.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when database updates or filesystem operations
    /// (rename, merge write) fail.
    pub async fn resolve_conflict(
        &self,
        folder: &str,
        local: &FileInfo,
        remote: &FileInfo,
        folder_path: &Path,
    ) -> Result<ConflictResolution> {
        debug!(
            folder = %folder,
            file = %local.name,
            local_version = ?local.version,
            remote_version = ?remote.version,
            "Checking for conflicts"
        );

        if !self.is_conflict(local, remote) {
            debug!(file = %local.name, "No conflict detected");
            return Ok(ConflictResolution::UseRemote);
        }

        info!(
            file = %local.name,
            "Conflict detected between local and remote versions"
        );

        self.events.publish(SyncEvent::ConflictDetected {
            folder: folder.to_string(),
            item: local.name.clone(),
            local_version: local.version.clone(),
            remote_version: remote.version.clone(),
        });

        // Local wins: keep the local lineage, the remote version is naturally
        // dominated, and no disk action is taken.
        if !self.remote_wins_conflict(local, remote) {
            info!(
                file = %local.name,
                "Conflict resolved deterministically: local wins"
            );
            self.db.update_file(folder, local.clone()).await?;
            self.events.publish(SyncEvent::ConflictResolved {
                folder: folder.to_string(),
                item: local.name.clone(),
                resolution: ConflictResolution::UseLocal,
            });
            return Ok(ConflictResolution::UseLocal);
        }

        // Remote wins: try auto-merge for text files, otherwise keep local
        // evidence (sync-conflict copy) and accept the remote version.
        let resolution = if crate::merge::is_mergeable_text(&local.name) {
            ConflictResolution::Merge
        } else {
            ConflictResolution::RenameBoth
        };

        if resolution == ConflictResolution::Merge {
            // Text merging is deferred to the Puller: accept the remote
            // version now so the Puller can run a true three-way merge
            // against base_version once the download completes.
            self.db.update_file(folder, remote.clone()).await?;
            self.events.publish(SyncEvent::ConflictResolved {
                folder: folder.to_string(),
                item: local.name.clone(),
                resolution,
            });
            return Ok(ConflictResolution::UseRemote);
        }

        self.apply_resolution(folder, local, remote, folder_path, resolution)
            .await?;

        self.events.publish(SyncEvent::ConflictResolved {
            folder: folder.to_string(),
            item: local.name.clone(),
            resolution,
        });

        Ok(resolution)
    }

    /// Returns `true` when the two version vectors are concurrent (a real conflict).
    pub fn is_conflict(&self, local: &FileInfo, remote: &FileInfo) -> bool {
        if local.version.dominates(&remote.version) {
            return false;
        }

        if remote.version.dominates(&local.version) {
            return false;
        }

        true
    }

    /// Applies the chosen conflict resolution.
    async fn apply_resolution(
        &self,
        folder: &str,
        local: &FileInfo,
        remote: &FileInfo,
        folder_path: &Path,
        resolution: ConflictResolution,
    ) -> Result<()> {
        match resolution {
            ConflictResolution::UseLocal => {
                debug!(file = %local.name, "Keeping local version");
                self.db.update_file(folder, local.clone()).await?;
            }
            ConflictResolution::UseRemote => {
                debug!(file = %local.name, "Using remote version");
                self.db.update_file(folder, remote.clone()).await?;
            }
            ConflictResolution::Merge => {
                debug!(file = %local.name, "Attempting merge");
                self.merge_files(folder, local, remote, folder_path).await?;
            }
            ConflictResolution::RenameBoth => {
                debug!(file = %local.name, "Renaming conflicting files");
                self.rename_conflict_files(folder, local, remote, folder_path)
                    .await?;
            }
        }

        Ok(())
    }

    /// Renames the local file to a Go-compatible conflict name and accepts the remote version.
    ///
    /// The conflict name is `name.sync-conflict-<timestamp>-<device short ID>.ext`,
    /// matching Go Syncthing's naming so cross-implementation conflict files
    /// stay recognizable.
    async fn rename_conflict_files(
        &self,
        folder: &str,
        local: &FileInfo,
        remote: &FileInfo,
        folder_path: &Path,
    ) -> Result<()> {
        let local_path = folder_path.join(&local.name);
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let device_tag = self
            .local_device_id
            .read()
            .ok()
            .and_then(|g| *g)
            .map(|id| id.short_id())
            .unwrap_or_else(|| "local".to_string());

        let (stem, ext) = match local.name.rsplit_once('.') {
            Some((s, e)) if !e.contains('/') && !e.contains('\\') => {
                (s.to_string(), format!(".{}", e))
            }
            _ => (local.name.clone(), String::new()),
        };
        let conflict_name = format!("{}.sync-conflict-{}-{}{}", stem, timestamp, device_tag, ext);
        let conflict_path = folder_path.join(&conflict_name);

        if local_path.exists() {
            fs::rename(&local_path, &conflict_path).await.map_err(|e| {
                SyncError::conflict(
                    local.name.clone(),
                    format!("Failed to rename local file: {}", e),
                )
            })?;
            info!(
                from = %local.name,
                to = %conflict_name,
                "Local file renamed as conflict"
            );
        }

        self.db.update_file(folder, remote.clone()).await?;

        Ok(())
    }

    /// Merges two text files (three-way merge).
    async fn merge_files(
        &self,
        folder: &str,
        local: &FileInfo,
        remote: &FileInfo,
        folder_path: &Path,
    ) -> Result<()> {
        use crate::merge::{is_mergeable_text, merge_text};

        if !is_mergeable_text(&local.name) {
            warn!(file = %local.name, "Not a mergeable text file, using rename strategy");
            return self
                .rename_conflict_files(folder, local, remote, folder_path)
                .await;
        }

        let local_path = folder_path.join(&local.name);

        let local_content = if local_path.exists() {
            fs::read_to_string(&local_path).await.ok()
        } else {
            None
        };

        let local_content = match local_content {
            Some(c) => c,
            None => {
                warn!(file = %local.name, "Cannot read local file as text, using rename strategy");
                return self
                    .rename_conflict_files(folder, local, remote, folder_path)
                    .await;
            }
        };

        let remote_path = folder_path.join(&remote.name);
        let remote_content = if remote_path.exists() {
            fs::read_to_string(&remote_path).await.ok()
        } else {
            None
        };

        let remote_content = match remote_content {
            Some(c) => c,
            None => {
                warn!(file = %local.name, "Cannot read remote file as text, using rename strategy");
                return self
                    .rename_conflict_files(folder, local, remote, folder_path)
                    .await;
            }
        };

        let merged = merge_text(&local_content, &remote_content, &local.name);

        fs::write(&local_path, &merged.content).await.map_err(|e| {
            SyncError::conflict(
                local.name.clone(),
                format!("Failed to write merged file: {}", e),
            )
        })?;

        if merged.has_conflicts {
            info!(
                file = %local.name,
                conflicts = merged.conflict_count,
                "Merged with conflicts (marked in file)"
            );
            self.events.publish(SyncEvent::ConflictResolved {
                folder: folder.to_string(),
                item: local.name.clone(),
                resolution: ConflictResolution::Merge,
            });
        } else {
            info!(file = %local.name, "Auto-merged without conflicts");
        }

        // Record the remote version; the merged file on disk is handled by the
        // puller in subsequent rounds.
        self.db.update_file(folder, remote.clone()).await?;

        Ok(())
    }

    /// Checks a batch of remote files for conflicts against the local database.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when a database lookup fails.
    pub async fn check_conflicts(
        &self,
        folder: &str,
        remote_files: &[FileInfo],
        _folder_path: &Path,
    ) -> Result<Vec<(FileInfo, FileInfo)>> {
        let mut conflicts = Vec::new();

        for remote in remote_files {
            if let Some(local) = self.db.get_file(folder, &remote.name).await? {
                if self.is_conflict(&local, remote) {
                    conflicts.push((local, remote.clone()));
                }
            }
        }

        Ok(conflicts)
    }

    /// Selects the winning version based on version vectors, falling back to mtime.
    pub fn select_winner<'a>(&self, local: &'a FileInfo, remote: &'a FileInfo) -> &'a FileInfo {
        if remote.version.dominates(&local.version) {
            return remote;
        }

        if local.version.dominates(&remote.version) {
            return local;
        }

        if remote.modified_s > local.modified_s {
            remote
        } else if remote.modified_s < local.modified_s {
            local
        } else if remote.modified_ns > local.modified_ns {
            remote
        } else {
            local
        }
    }

    /// Compares two version vectors.
    pub fn compare_versions(&self, v1: &Vector, v2: &Vector) -> VersionComparison {
        let v1_dominates = v1.dominates(v2);
        let v2_dominates = v2.dominates(v1);

        if v1_dominates && !v2_dominates {
            VersionComparison::Greater
        } else if v2_dominates && !v1_dominates {
            VersionComparison::Less
        } else if v1_dominates && v2_dominates {
            VersionComparison::Equal
        } else {
            VersionComparison::Conflict
        }
    }
}

/// Result of comparing two version vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionComparison {
    /// The vectors are identical.
    Equal,
    /// The first vector dominates the second.
    Greater,
    /// The second vector dominates the first.
    Less,
    /// The vectors are concurrent.
    Conflict,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryDatabase;

    fn create_test_file(name: &str, version: Vector) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            file_type: syncthing_core::types::FileType::File,
            size: 100,
            permissions: 0o644,
            modified_s: 1234567890,
            modified_ns: 0,
            version,
            sequence: 1,
            block_size: 128 * 1024,
            blocks: vec![],
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        }
    }

    #[test]
    fn test_version_comparison() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let resolver = ConflictResolver::new(db, events);

        let v1 = Vector::new().with_counter(1, 5);
        let v2 = Vector::new().with_counter(1, 3);
        let v3 = Vector::new().with_counter(2, 4);

        assert_eq!(
            resolver.compare_versions(&v1, &v2),
            VersionComparison::Greater
        );
        assert_eq!(resolver.compare_versions(&v2, &v1), VersionComparison::Less);
        assert_eq!(
            resolver.compare_versions(&v1, &v3),
            VersionComparison::Conflict
        );
    }

    #[test]
    fn test_conflict_detection() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let resolver = ConflictResolver::new(db, events);

        let local = create_test_file("test.txt", Vector::new().with_counter(1, 5));
        let remote = create_test_file("test.txt", Vector::new().with_counter(2, 3));

        assert!(resolver.is_conflict(&local, &remote));
    }

    #[test]
    fn test_no_conflict_when_dominates() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let resolver = ConflictResolver::new(db, events);

        let local = create_test_file("test.txt", Vector::new().with_counter(1, 5));
        let remote = create_test_file("test.txt", Vector::new().with_counter(1, 3));

        assert!(!resolver.is_conflict(&local, &remote));
    }
}

#[cfg(test)]
mod convergence_tests {
    use super::*;
    use crate::database::MemoryDatabase;

    fn file_with(name: &str, version: Vector, mtime: i64) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            file_type: syncthing_core::types::FileType::File,
            size: 100,
            permissions: 0o644,
            modified_s: mtime,
            modified_ns: 0,
            version,
            sequence: 1,
            block_size: 128 * 1024,
            blocks: vec![],
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        }
    }

    /// Phase 3 核心（死循环死亡证明）：独立谱系并发冲突，
    /// 双端各自 resolve 后必须收敛到同一胜者、同一版本向量。
    #[tokio::test]
    async fn test_concurrent_conflict_converges_same_winner() {
        // B 侧视角：本地 {2:5}（B 谱系，mtime 旧） vs 远端 {1:5}（A 谱系，mtime 新）
        let dir_b = tempfile::tempdir().unwrap();
        std::fs::write(dir_b.path().join("x.bin"), b"B content").unwrap();
        let db_b = MemoryDatabase::new();
        let resolver_b = ConflictResolver::new(db_b.clone(), EventPublisher::new(10));
        resolver_b.set_local_device_id(syncthing_core::DeviceId::from_bytes(&[2u8; 32]).unwrap());
        let local_b = file_with("x.bin", Vector::new().with_counter(2, 5), 1000);
        let remote_b = file_with("x.bin", Vector::new().with_counter(1, 5), 2000);
        let res_b = resolver_b
            .resolve_conflict("f", &local_b, &remote_b, dir_b.path())
            .await
            .unwrap();

        // A 侧视角：本地 {1:5}（mtime 新） vs 远端 {2:5}（mtime 旧）
        let dir_a = tempfile::tempdir().unwrap();
        std::fs::write(dir_a.path().join("x.bin"), b"A content").unwrap();
        let db_a = MemoryDatabase::new();
        let resolver_a = ConflictResolver::new(db_a.clone(), EventPublisher::new(10));
        let local_a = file_with("x.bin", Vector::new().with_counter(1, 5), 2000);
        let remote_a = file_with("x.bin", Vector::new().with_counter(2, 5), 1000);
        let res_a = resolver_a
            .resolve_conflict("f", &local_a, &remote_a, dir_a.path())
            .await
            .unwrap();

        // B 侧：远程胜 → 本地留证（sync-conflict 含设备短 ID）+ DB 接受 {1:5}
        assert_eq!(res_b, ConflictResolution::RenameBoth);
        let b_stored = db_b.get_file("f", "x.bin").await.unwrap().expect("entry");
        assert_eq!(b_stored.version.get(1), 5, "B 侧必须接受胜者谱系");
        let conflict_exists = std::fs::read_dir(dir_b.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.contains("sync-conflict") && n.contains("0202020202020202")
            });
        assert!(conflict_exists, "冲突副本必须包含本地设备短 ID");

        // A 侧：本地胜 → 保留本地谱系 {1:5}，无磁盘动作
        assert_eq!(res_a, ConflictResolution::UseLocal);
        let a_stored = db_a.get_file("f", "x.bin").await.unwrap().expect("entry");
        assert_eq!(a_stored.version.get(1), 5);

        // 收敛断言：双端版本向量一致，且不再并发（下一轮不会重演冲突）
        assert_eq!(a_stored.version, b_stored.version, "双端必须收敛到同一版本");
        assert_eq!(
            a_stored.version.concurrent_order(&b_stored.version),
            None,
            "收敛后不得再并发"
        );
    }

    /// mtime 相等时按并发方向决胜，且双端独立计算结果一致
    #[tokio::test]
    async fn test_conflict_tiebreak_by_vector_direction() {
        let db = MemoryDatabase::new();
        let resolver = ConflictResolver::new(db, EventPublisher::new(10));

        // 远端 {2:5} vs 本地 {1:6}：id1 远端 0<6（OtherGreater 先），
        // id2 远端 5>0（SelfGreater）→ 并发，方向 OtherGreater → 远端不赢
        assert!(!resolver.remote_wins_conflict(
            &file_with("x", Vector::new().with_counter(1, 6), 100),
            &file_with("x", Vector::new().with_counter(2, 5), 100),
        ));

        // 对偶视角（远端 {1:6} vs 本地 {2:5}）：远端赢。
        // 同一谱系 {1:6} 在两端都被判为胜者 → 收敛
        assert!(resolver.remote_wins_conflict(
            &file_with("x", Vector::new().with_counter(2, 5), 100),
            &file_with("x", Vector::new().with_counter(1, 6), 100),
        ));
    }
}
