//! 冲突解决器
//! 
//! 处理文件版本冲突，实现 Syncthing 的冲突解决策略

use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{ConflictResolution, EventPublisher, SyncEvent};
use syncthing_core::types::{FileInfo, Vector};
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, info, warn};

/// 冲突解决器
pub struct ConflictResolver {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
}

impl ConflictResolver {
    /// 创建新的冲突解决器
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher) -> Self {
        Self { db, events }
    }

    /// 检查并解决冲突
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

        // 检查是否真的是冲突（版本向量不可比较）
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

        // 默认策略：重命名保留双方修改
        let resolution = ConflictResolution::RenameBoth;
        self.apply_resolution(folder, local, remote, folder_path, resolution).await?;

        self.events.publish(SyncEvent::ConflictResolved {
            folder: folder.to_string(),
            item: local.name.clone(),
            resolution,
        });

        Ok(resolution)
    }

    /// 检查是否为冲突
    pub fn is_conflict(&self, local: &FileInfo, remote: &FileInfo) -> bool {
        // 如果本地版本支配远程版本，没有冲突
        if local.version.dominates(&remote.version) {
            return false;
        }
        
        // 如果远程版本支配本地版本，没有冲突
        if remote.version.dominates(&local.version) {
            return false;
        }

        // 版本向量不可比较，存在冲突
        true
    }

    /// 应用冲突解决方案
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
                // 保留本地版本，发送给远程
                debug!(file = %local.name, "Keeping local version");
                // 更新数据库中的版本
                self.db.update_file(folder, local.clone()).await?;
            }
            ConflictResolution::UseRemote => {
                // 使用远程版本
                debug!(file = %local.name, "Using remote version");
                self.db.update_file(folder, remote.clone()).await?;
            }
            ConflictResolution::Merge => {
                // 尝试合并（仅对文本文件有效）
                debug!(file = %local.name, "Attempting merge");
                self.merge_files(folder, local, remote, folder_path).await?;
            }
            ConflictResolution::RenameBoth => {
                // 重命名保留双方修改
                debug!(file = %local.name, "Renaming conflicting files");
                self.rename_conflict_files(folder, local, remote, folder_path).await?;
            }
        }

        Ok(())
    }

    /// 重命名冲突文件
    async fn rename_conflict_files(
        &self,
        folder: &str,
        local: &FileInfo,
        remote: &FileInfo,
        folder_path: &Path,
    ) -> Result<()> {
        let local_path = folder_path.join(&local.name);
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        
        // 生成冲突文件名
        let conflict_name = format!(
            "{}.sync-conflict-{}-{}",
            local.name,
            timestamp,
            "local" // 实际应该使用设备短ID
        );
        let conflict_path = folder_path.join(&conflict_name);

        // 如果本地文件存在，重命名为冲突文件
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

        // 接受远程版本
        self.db.update_file(folder, remote.clone()).await?;

        Ok(())
    }

    /// 合并文件（简化实现）
    async fn merge_files(
        &self,
        folder: &str,
        local: &FileInfo,
        remote: &FileInfo,
        folder_path: &Path,
    ) -> Result<()> {
        let local_path = folder_path.join(&local.name);
        
        // 读取本地文件内容
        let local_content = if local_path.exists() {
            fs::read_to_string(&local_path).await.ok()
        } else {
            None
        };

        // 如果无法读取为文本，回退到重命名策略
        if local_content.is_none() {
            warn!(file = %local.name, "Cannot merge binary file, using rename strategy");
            return self.rename_conflict_files(folder, local, remote, folder_path).await;
        }

        // 简化的合并策略：使用远程版本并标记为冲突
        // 实际实现应该使用三向合并或类似算法
        self.rename_conflict_files(folder, local, remote, folder_path).await
    }

    /// 批量检查冲突
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

    /// 选择获胜版本（基于版本向量）
    pub fn select_winner<'a>(&self, local: &'a FileInfo, remote: &'a FileInfo) -> &'a FileInfo {
        // 如果远程版本支配本地，选择远程
        if remote.version.dominates(&local.version) {
            return remote;
        }
        
        // 如果本地版本支配远程，选择本地
        if local.version.dominates(&remote.version) {
            return local;
        }

        // 冲突情况：比较修改时间
        if remote.modified_s > local.modified_s {
            remote
        } else if remote.modified_s < local.modified_s {
            local
        } else {
            // 修改时间相同，比较纳秒
            if remote.modified_ns > local.modified_ns {
                remote
            } else {
                local
            }
        }
    }

    /// 比较两个版本向量
    pub fn compare_versions(&self, v1: &Vector, v2: &Vector) -> VersionComparison {
        let v1_dominates = v1.dominates(v2);
        let v2_dominates = v2.dominates(v1);

        if v1_dominates && !v2_dominates {
            VersionComparison::Greater
        } else if v2_dominates && !v1_dominates {
            VersionComparison::Less
        } else if v1_dominates && v2_dominates {
            // 这种情况不应该发生
            VersionComparison::Equal
        } else {
            VersionComparison::Conflict
        }
    }
}

/// 版本比较结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionComparison {
    Equal,
    Greater,
    Less,
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

        assert_eq!(resolver.compare_versions(&v1, &v2), VersionComparison::Greater);
        assert_eq!(resolver.compare_versions(&v2, &v1), VersionComparison::Less);
        assert_eq!(resolver.compare_versions(&v1, &v3), VersionComparison::Conflict);
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
