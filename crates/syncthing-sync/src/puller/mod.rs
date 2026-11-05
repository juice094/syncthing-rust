//! File puller.
//!
//! Downloads files from remote devices.
//!
//! See "docs/design/ADR-20261105-pull-delete-quota.md" and
//! "docs/design/ADR-20261105-remote-deletion-guards.md".

pub mod concurrency;
pub(crate) mod ops;
pub(crate) mod rename;
pub use concurrency::{ConcurrencyPolicy, RttTracker};
use ops::{content_hash, find_local_copy_source, recover_base_content};
use rename::rename_with_retry;
pub(crate) use rename::temp_path_for;

use crate::block_server::validate_remote_name;
use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{EventPublisher, ItemAction, SyncEvent};
use crate::merge::{is_mergeable_text, three_way_merge};
use bytes::Bytes;
use sha2::Digest;
use std::path::Path;
use std::sync::{Arc, RwLock};
use syncthing_core::types::{BlockInfo, FileInfo, FileInfoBase, FileType, Folder};
use syncthing_versioner::Versioner;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, trace, warn};

/// Source of file blocks for downloads.
#[async_trait::async_trait]
pub trait BlockSource: Send + Sync {
    /// Requests one block of a file from a peer.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when the block cannot be fetched (e.g. the peer
    /// no longer has the file).
    async fn request_block(
        &self,
        folder: &str,
        file: &str,
        block: &BlockInfo,
        block_no: usize,
    ) -> Result<Bytes>;
}

/// Downloads needed files from remote devices.
pub struct Puller {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    max_concurrent_downloads: usize,
    max_concurrent_blocks: usize,
    concurrency_policy: RwLock<Option<Arc<ConcurrencyPolicy>>>,
    block_source: RwLock<Option<Arc<dyn BlockSource>>>,
    versioner: Option<Arc<dyn Versioner>>,
}

impl Puller {
    /// Creates a new puller.
    ///
    /// Defaults to a conservative 2 concurrent file downloads (friendly to
    /// high-latency links) and 4 concurrent block requests per file (avoids
    /// head-of-line blocking).
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher) -> Self {
        Self {
            db,
            events,
            max_concurrent_downloads: 2,
            max_concurrent_blocks: 4,
            concurrency_policy: RwLock::new(None),
            block_source: RwLock::new(None),
            versioner: None,
        }
    }

    /// Sets the max concurrent downloads (file level, builder style).
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent_downloads = max;
        self
    }

    /// Sets the max concurrent block requests within one file (builder style).
    pub fn with_max_concurrent_blocks(mut self, max: usize) -> Self {
        self.max_concurrent_blocks = max;
        self
    }

    /// Sets the shared concurrency policy (builder style).
    pub fn with_concurrency_policy(mut self, policy: Option<Arc<ConcurrencyPolicy>>) -> Self {
        self.concurrency_policy = RwLock::new(policy);
        self
    }

    /// Updates the shared concurrency policy at runtime.
    pub fn set_concurrency_policy(&self, policy: Option<Arc<ConcurrencyPolicy>>) {
        if let Ok(mut guard) = self.concurrency_policy.write() {
            *guard = policy;
        }
    }

    /// Sets the block source (builder style).
    pub fn with_block_source(mut self, source: Option<Arc<dyn BlockSource>>) -> Self {
        self.block_source = RwLock::new(source);
        self
    }

    /// Updates the block source at runtime (allows wiring it in after
    /// `add_folder`, fixing the previous read-only-at-creation defect).
    pub fn set_block_source(&self, source: Option<Arc<dyn BlockSource>>) {
        if let Ok(mut guard) = self.block_source.write() {
            *guard = source;
        }
    }

    /// 测试辅助：是否已配置块数据源
    #[cfg(test)]
    pub(crate) fn block_source_present(&self) -> bool {
        self.block_source
            .read()
            .map(|g| g.is_some())
            .unwrap_or(false)
    }

    /// Sets the versioner used to archive overwritten/deleted files (builder style).
    pub fn with_versioner(mut self, versioner: Option<Arc<dyn Versioner>>) -> Self {
        self.versioner = versioner;
        self
    }

    /// Pulls all needed files of a folder from remote devices.
    ///
    /// Applies the per-pull remote-deletion quota before processing (see
    /// `docs/design/ADR-20261105-pull-delete-quota.md`).
    ///
    /// Failures are counted, never retried inline: failed files are retried
    /// with backoff by the folder's pull loop, and correctness relies on
    /// `check_needed_files()` re-deriving the needed set each round.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when the target directory cannot be created.
    /// Individual file failures are counted in [`PullStats`] and do not
    /// abort the pull.
    pub async fn pull_folder(
        &self,
        folder: &Folder,
        needed_files: Vec<FileInfo>,
    ) -> Result<PullStats> {
        info!(folder_id = %folder.id, file_count = needed_files.len(), "Starting folder pull");

        let mut stats = PullStats::default();
        let base_path = Path::new(&folder.path);

        fs::create_dir_all(&base_path).await.map_err(|e| {
            SyncError::pull(
                folder.path.clone(),
                format!("Failed to create directory: {}", e),
            )
        })?;

        // Per-pull remote-deletion quota (see the module-level ADR pointer):
        // an over-quota deletion batch is dropped wholesale; downloads and
        // modifications are unaffected.
        let local_count = self
            .db
            .get_folder_files(&folder.id)
            .await?
            .iter()
            .filter(|f| !f.is_deleted())
            .count();
        let quota = syncthing_core::constants::PULL_DELETE_QUOTA_MIN.max(
            (local_count as f64 * syncthing_core::constants::PULL_DELETE_QUOTA_RATIO) as usize,
        );
        let delete_count = needed_files.iter().filter(|f| f.is_deleted()).count();
        let needed_files = if delete_count > quota {
            error!(
                folder_id = %folder.id,
                delete_count,
                quota,
                local_files = local_count,
                "SAFETY QUOTA: refusing remote deletion batch; peer index may be stale/corrupt. \
                 Investigate the peer before allowing these deletions."
            );
            needed_files
                .into_iter()
                .filter(|f| !f.is_deleted())
                .collect::<Vec<_>>()
        } else {
            needed_files
        };

        Self::cleanup_sttrash(base_path).await;

        let policy = self.concurrency_policy.read().ok().and_then(|g| g.clone());
        let max_concurrent_downloads = policy
            .as_ref()
            .map(|p| p.downloads())
            .unwrap_or(self.max_concurrent_downloads);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_downloads));
        let mut handles = Vec::new();

        for file_info in needed_files {
            let permit = semaphore.clone().acquire_owned().await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to acquire permit: {}", e),
                )
            })?;

            let db = self.db.clone();
            let events = self.events.clone();
            let folder_id = folder.id.clone();
            let folder_path = base_path.to_path_buf();
            let block_source = self.block_source.read().ok().and_then(|g| g.clone());
            let max_concurrent_blocks = policy
                .as_ref()
                .map(|p| p.blocks())
                .unwrap_or(self.max_concurrent_blocks);
            let versioner = self.versioner.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;

                events.publish(SyncEvent::ItemStarted {
                    folder: folder_id.clone(),
                    item: file_info.name.clone(),
                    action: if file_info.is_deleted() {
                        ItemAction::Delete
                    } else {
                        ItemAction::Modify
                    },
                });

                let result = if file_info.is_deleted() {
                    Self::delete_file(&folder_path, &file_info, &*db, &folder_id).await
                } else {
                    match file_info.file_type {
                        FileType::Directory => {
                            Self::create_directory(&folder_path, &file_info).await
                        }
                        FileType::File => {
                            Self::download_file(
                                &folder_path,
                                &file_info,
                                &*db,
                                &events,
                                &folder_id,
                                block_source,
                                max_concurrent_blocks,
                                versioner.as_ref(),
                            )
                            .await
                        }
                        FileType::Symlink => {
                            warn!(file = %file_info.name, "Symlink sync not yet implemented, skipping");
                            Ok(())
                        }
                    }
                };

                match &result {
                    Ok(_) => {
                        events.publish(SyncEvent::ItemFinished {
                            folder: folder_id,
                            item: file_info.name.clone(),
                            action: if file_info.is_deleted() {
                                ItemAction::Delete
                            } else {
                                ItemAction::Modify
                            },
                            error: None,
                        });
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        events.publish(SyncEvent::ItemFinished {
                            folder: folder_id,
                            item: file_info.name.clone(),
                            action: if file_info.is_deleted() {
                                ItemAction::Delete
                            } else {
                                ItemAction::Modify
                            },
                            error: Some(err_str),
                        });
                    }
                }

                result
            });

            handles.push(handle);
        }

        let mut stale_index_errors = 0usize;
        for handle in handles {
            match handle.await {
                Ok(Ok(_)) => {
                    stats.files_succeeded += 1;
                }
                Ok(Err(e)) => {
                    // ponytail: BEP error code 2 (NO_SUCH_FILE) reaches us as a
                    // formatted string from the bridge layer, so we count by
                    // substring; structured error codes would require changing
                    // the bridge signature, which is not worth the benefit.
                    if e.to_string().contains("error code 2") {
                        stale_index_errors += 1;
                    }
                    error!(error = %e, "File pull failed");
                    stats.files_failed += 1;
                }
                Err(e) => {
                    error!(error = %e, "Task join failed");
                    stats.files_failed += 1;
                }
            }
        }

        if stale_index_errors > syncthing_core::constants::STALE_INDEX_WARN_THRESHOLD {
            warn!(
                folder_id = %folder.id,
                count = stale_index_errors,
                "Many NO_SUCH_FILE (error code 2) responses: peer index may be stale/corrupt. \
                 Recommend a full rescan on the peer before further sync."
            );
        }

        info!(
            folder_id = %folder.id,
            succeeded = stats.files_succeeded,
            failed = stats.files_failed,
            "Folder pull completed"
        );

        Ok(stats)
    }

    /// Downloads a single file into a temporary file, then atomically renames
    /// it into place.
    ///
    /// Block requests run concurrently (bounded by `max_concurrent_blocks`)
    /// but are written to disk in order.
    #[allow(clippy::too_many_arguments)]
    async fn download_file(
        folder_path: &Path,
        file_info: &FileInfo,
        db: &dyn LocalDatabase,
        events: &EventPublisher,
        folder_id: &str,
        block_source: Option<Arc<dyn BlockSource>>,
        max_concurrent_blocks: usize,
        versioner: Option<&Arc<dyn Versioner>>,
    ) -> Result<()> {
        // Path-traversal defense: reject malicious names from the peer.
        validate_remote_name(&file_info.name).map_err(|e| {
            SyncError::pull(
                file_info.name.clone(),
                format!("Invalid remote file name (path traversal rejected): {}", e),
            )
        })?;

        debug!(file = %file_info.name, size = file_info.size, blocks = file_info.blocks.len(), max_concurrent = max_concurrent_blocks, "Downloading file");

        let file_path = folder_path.join(&file_info.name);
        let temp_path = temp_path_for(&file_path);

        async fn cleanup_temp(path: &Path) {
            if path.exists() {
                if let Err(e) = fs::remove_file(path).await {
                    warn!(path = %path.display(), error = %e, "Failed to cleanup temp file");
                } else {
                    debug!(path = %path.display(), "Cleaned up temp file after failed download");
                }
            }
        }

        if let Some(source_path) =
            find_local_copy_source(folder_path, file_info, db, folder_id).await?
        {
            info!(file = %file_info.name, source = %source_path.display(), "Copying from local file (rename optimization)");
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    SyncError::pull(
                        file_info.name.clone(),
                        format!("Failed to create parent directory: {}", e),
                    )
                })?;
            }
            fs::copy(&source_path, &temp_path).await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to copy from local source: {}", e),
                )
            })?;
            if let Some(v) = versioner {
                if file_path.exists() {
                    let _ = v.archive(&file_path).await;
                }
            }
            rename_with_retry(&temp_path, &file_path, &file_info.name).await?;
            let modified = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(file_info.modified_s as u64)
                + std::time::Duration::from_nanos(file_info.modified_ns as u64);
            let mtime = filetime::FileTime::from_system_time(modified);
            if let Err(e) = filetime::set_file_mtime(&file_path, mtime) {
                warn!(file = %file_info.name, error = %e, "Failed to set file modification time");
            }
            if let Err(e) = db.update_file(folder_id, file_info.clone()).await {
                warn!(file = %file_info.name, error = %e, "Failed to update database after local copy");
            }
            return Ok(());
        }

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to create parent directory: {}", e),
                )
            })?;
        }

        let mut file = fs::File::create(&temp_path).await.map_err(|e| {
            SyncError::pull(
                file_info.name.clone(),
                format!("Failed to create temp file: {}", e),
            )
        })?;

        let mut bytes_downloaded = 0u64;

        let block_semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_blocks));
        let mut handles = Vec::with_capacity(file_info.blocks.len());

        for (idx, block) in file_info.blocks.iter().enumerate() {
            let permit = block_semaphore.clone().acquire_owned().await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to acquire block permit: {}", e),
                )
            })?;

            let source = match &block_source {
                Some(s) => s.clone(),
                None => {
                    cleanup_temp(&temp_path).await;
                    return Err(SyncError::pull(
                        file_info.name.clone(),
                        "No block source configured".to_string(),
                    ));
                }
            };

            let folder_id = folder_id.to_string();
            let file_name = file_info.name.clone();
            let block = block.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;
                let data = source
                    .request_block(&folder_id, &file_name, &block, idx)
                    .await?;
                Ok::<_, SyncError>((idx, block.offset, data, block.hash))
            });
            handles.push(handle);
        }

        for handle in handles {
            let (idx, offset, block_data, expected_hash) = match handle.await {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => {
                    error!(file = %file_info.name, error = %e, "Block download failed");
                    cleanup_temp(&temp_path).await;
                    return Err(e);
                }
                Err(e) => {
                    error!(file = %file_info.name, error = %e, "Block download task panicked");
                    cleanup_temp(&temp_path).await;
                    return Err(SyncError::pull(
                        file_info.name.clone(),
                        format!("Block download task panicked: {}", e),
                    ));
                }
            };

            trace!(file = %file_info.name, block = idx, offset = offset, size = block_data.len(), "Writing block");

            let hash = sha2::Sha256::digest(&block_data);
            if hash.as_slice() != expected_hash.as_slice() {
                cleanup_temp(&temp_path).await;
                return Err(SyncError::ChecksumMismatch { offset });
            }

            if let Err(e) = file.write_all(&block_data).await {
                cleanup_temp(&temp_path).await;
                return Err(SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to write block: {}", e),
                ));
            }

            bytes_downloaded += block_data.len() as u64;

            events.publish(SyncEvent::DownloadProgress {
                folder: folder_id.to_string(),
                file: file_info.name.clone(),
                bytes_done: bytes_downloaded,
                bytes_total: file_info.size as u64,
            });
        }

        if let Err(e) = file.flush().await {
            cleanup_temp(&temp_path).await;
            return Err(SyncError::pull(
                file_info.name.clone(),
                format!("Failed to flush file: {}", e),
            ));
        }
        drop(file);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(file_info.permissions);
            if let Err(e) = fs::set_permissions(&temp_path, perms).await {
                cleanup_temp(&temp_path).await;
                return Err(SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to set permissions: {}", e),
                ));
            }
        }

        let mut merged_info = file_info.clone();
        let local_info = db.get_file(folder_id, &file_info.name).await.ok().flatten();
        debug!(
            file = %file_info.name,
            exists = file_path.exists(),
            has_base = local_info.as_ref().and_then(|i| i.base_version.as_ref()).is_some(),
            has_versioner = versioner.is_some(),
            "Checking merge preconditions"
        );
        if file_path.exists() {
            if let Some(base) = local_info.as_ref().and_then(|i| i.base_version.as_ref()) {
                if is_mergeable_text(&file_info.name) {
                    if let (Ok(local_content), Ok(remote_content)) = (
                        fs::read_to_string(&file_path).await,
                        fs::read_to_string(&temp_path).await,
                    ) {
                        if let Some(v) = versioner {
                            debug!("Attempting base content recovery for three-way merge");
                            if let Some(base_content) =
                                recover_base_content(&**v, &file_path, base).await
                            {
                                debug!("Base recovered, performing three-way merge");
                                let merged = three_way_merge(
                                    &base_content,
                                    &local_content,
                                    &remote_content,
                                    &file_info.name,
                                );
                                debug!(
                                    file = %file_info.name,
                                    merged_len = merged.content.len(),
                                    conflicts = merged.conflict_count,
                                    "Three-way merge result computed"
                                );
                                if let Err(e) = fs::write(&temp_path, &merged.content).await {
                                    warn!(file = %file_info.name, error = %e, "Failed to write merged content");
                                } else {
                                    info!(
                                        file = %file_info.name,
                                        conflicts = merged.conflict_count,
                                        has_conflicts = merged.has_conflicts,
                                        "Three-way merge completed"
                                    );
                                    let hash = content_hash(merged.content.as_bytes());
                                    merged_info.base_version = Some(FileInfoBase {
                                        size: merged.content.len() as i64,
                                        modified_s: file_info.modified_s,
                                        modified_ns: file_info.modified_ns,
                                        blocks_hash: None,
                                        content_hash: Some(hash),
                                    });
                                }
                            } else {
                                warn!(file = %file_info.name, "Failed to recover base content for merge");
                            }
                        }
                    }
                }
            }
        }

        if let Some(v) = versioner {
            if file_path.exists() {
                if let Err(e) = v.archive(&file_path).await {
                    warn!(file = %file_info.name, error = %e, "Failed to archive old version");
                }
            }
        }

        rename_with_retry(&temp_path, &file_path, &file_info.name).await?;

        let modified = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(file_info.modified_s as u64)
            + std::time::Duration::from_nanos(file_info.modified_ns as u64);

        let mtime = filetime::FileTime::from_system_time(modified);
        if let Err(e) = filetime::set_file_mtime(&file_path, mtime) {
            warn!(
                file = %file_info.name,
                error = %e,
                "Failed to set file modification time"
            );
        }

        if let Err(e) = db.update_file(folder_id, merged_info).await {
            warn!(file = %file_info.name, error = %e, "Failed to update database after download");
        }

        info!(file = %file_info.name, "File download completed");
        Ok(())
    }

    /// Creates a directory (idempotent when it already exists).
    async fn create_directory(folder_path: &Path, file_info: &FileInfo) -> Result<()> {
        validate_remote_name(&file_info.name).map_err(|e| {
            SyncError::pull(
                file_info.name.clone(),
                format!(
                    "Invalid remote directory name (path traversal rejected): {}",
                    e
                ),
            )
        })?;

        let dir_path = folder_path.join(&file_info.name);

        if dir_path.exists() {
            if !dir_path.is_dir() {
                return Err(SyncError::pull(
                    file_info.name.clone(),
                    format!("Path exists but is not a directory: {}", dir_path.display()),
                ));
            }
        } else {
            fs::create_dir_all(&dir_path).await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to create directory: {}", e),
                )
            })?;
            info!(dir = %file_info.name, "Directory created");
        }

        Ok(())
    }

    /// Deletes a file, quarantining it into `.sttrash` first (soft delete).
    ///
    /// Remote-driven deletions are quarantined rather than unlinked so that a
    /// corrupted peer index cannot destroy local data irreversibly (see
    /// "docs/design/ADR-20261105-remote-deletion-guards.md").
    async fn delete_file(
        folder_path: &Path,
        file_info: &FileInfo,
        db: &dyn LocalDatabase,
        folder_id: &str,
    ) -> Result<()> {
        validate_remote_name(&file_info.name).map_err(|e| {
            SyncError::pull(
                file_info.name.clone(),
                format!(
                    "Invalid remote file name for deletion (path traversal rejected): {}",
                    e
                ),
            )
        })?;

        debug!(file = %file_info.name, "Deleting file");

        let file_path = folder_path.join(&file_info.name);

        if file_path.exists() {
            // Soft delete: quarantine into .sttrash so a corrupted peer index
            // cannot destroy local data irreversibly (see delete_file docs).
            match Self::move_to_trash(&file_path, folder_path, &file_info.name).await {
                Ok(trash_path) => {
                    info!(
                        file = %file_info.name,
                        trash = %trash_path.display(),
                        "File moved to .sttrash (remote deletion applied)"
                    );
                }
                Err(e) => {
                    return Err(SyncError::pull(
                        file_info.name.clone(),
                        format!("Failed to quarantine file to .sttrash: {}", e),
                    ));
                }
            }
        } else {
            warn!(file = %file_info.name, "File to delete not found");
        }

        db.update_file(folder_id, file_info.clone()).await?;

        Ok(())
    }

    /// Moves the target into `<folder>/.sttrash/<YYYYMMDD>/<relative path>`,
    /// appending a timestamp on name collisions. Returns the final trash path.
    async fn move_to_trash(
        file_path: &Path,
        folder_path: &Path,
        rel_name: &str,
    ) -> Result<std::path::PathBuf> {
        let date = chrono::Utc::now().format("%Y%m%d").to_string();
        let trash_dir = folder_path.join(".sttrash").join(&date);
        let mut dest = trash_dir.join(rel_name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SyncError::pull(
                    rel_name.to_string(),
                    format!("Failed to create .sttrash dir: {}", e),
                )
            })?;
        }
        if dest.exists() {
            let ts = chrono::Utc::now().format("%H%M%S").to_string();
            let file_name = dest
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            dest.set_file_name(format!("{}.{}", file_name, ts));
        }
        fs::rename(file_path, &dest).await.map_err(|e| {
            SyncError::pull(
                rel_name.to_string(),
                format!("Failed to move to .sttrash: {}", e),
            )
        })?;
        Ok(dest)
    }

    /// Removes `.sttrash` date directories older than the 7-day retention.
    ///
    /// ponytail: dates are judged by directory name only (no recursive content
    /// checks); cleanup failures are warned about, never fatal.
    async fn cleanup_sttrash(folder_path: &Path) {
        let trash_root = folder_path.join(".sttrash");
        let mut entries = match fs::read_dir(&trash_root).await {
            Ok(e) => e,
            // No trash directory: normal path.
            Err(_) => return,
        };
        let cutoff = chrono::Utc::now() - chrono::Duration::days(7);
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(date) = chrono::NaiveDate::parse_from_str(&name, "%Y%m%d") else {
                continue;
            };
            let Some(datetime) = date.and_hms_opt(0, 0, 0) else {
                continue;
            };
            if chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(datetime, chrono::Utc)
                < cutoff
            {
                info!(dir = %entry.path().display(), "Removing expired .sttrash directory");
                if let Err(e) = fs::remove_dir_all(entry.path()).await {
                    warn!(dir = %entry.path().display(), "Failed to remove expired .sttrash: {}", e);
                }
            }
        }
    }

    /// Re-derives the set of files that still need to be pulled.
    ///
    /// A file is needed when it is marked deleted in the DB but still exists
    /// on disk, or when it is missing on disk, or when its size differs.
    /// Directories only need to exist and are not content-checked.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when a database lookup or metadata read fails.
    pub async fn check_needed_files(&self, folder: &Folder) -> Result<Vec<FileInfo>> {
        let db_files: Vec<syncthing_core::types::FileInfo> =
            self.db.get_folder_files(&folder.id).await?;
        let base_path = Path::new(&folder.path);
        let mut needed = Vec::new();
        let mut dirs_unchanged = 0u64;

        for file_info in db_files {
            if file_info.is_deleted() {
                let file_path = base_path.join(&file_info.name);
                if file_path.exists() {
                    needed.push(file_info);
                }
            } else {
                let file_path = base_path.join(&file_info.name);
                if !file_path.exists() {
                    needed.push(file_info);
                } else if file_info.file_type == FileType::Directory {
                    dirs_unchanged += 1;
                } else {
                    let metadata = fs::metadata(&file_path).await?;
                    if metadata.len() != file_info.size as u64 {
                        needed.push(file_info);
                    }
                }
            }
        }

        if dirs_unchanged > 0 {
            trace!(folder_id = %folder.id, count = dirs_unchanged, "Existing directories need no update");
        }

        Ok(needed)
    }
}

/// Per-folder pull statistics.
#[derive(Debug, Clone, Default)]
pub struct PullStats {
    /// Number of files pulled successfully.
    pub files_succeeded: usize,
    /// Number of files that failed (retried later by the folder pull loop).
    pub files_failed: usize,
    /// Total bytes transferred.
    pub bytes_transferred: u64,
}

#[cfg(test)]
mod tests;
