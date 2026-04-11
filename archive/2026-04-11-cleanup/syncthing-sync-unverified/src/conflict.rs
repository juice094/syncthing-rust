//! Module: syncthing-sync
//! Worker: Agent-C
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! 冲突检测与解决模块
//!
//! 该模块实现 Syncthing 的冲突检测和解决机制，主要基于版本向量（Version Vector）
//! 来检测并发更新冲突，并提供 "last writer wins" 的冲突解决策略。

use std::cmp::Ordering;
use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use syncthing_core::types::{FileInfo, FolderId};
use syncthing_core::version_vector::VersionVector;
use tracing::{debug, info, warn};

/// 冲突检测结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    /// 本地版本胜出（本地版本更新）
    LocalWins,
    /// 远程版本胜出（远程版本更新）
    RemoteWins,
    /// 检测到冲突，需要生成冲突副本
    Conflict {
        /// 冲突副本的文件路径
        conflict_copy_path: String,
    },
}

/// 冲突解决器
#[derive(Debug, Clone)]
pub struct ConflictResolver {
    /// 本设备 ID
    local_device: syncthing_core::types::DeviceId,
}

impl ConflictResolver {
    /// 创建新的冲突解决器
    pub fn new(local_device: syncthing_core::types::DeviceId) -> Self {
        Self { local_device }
    }

    /// 比较本地和远程文件版本，决定是否需要同步以及如何处理冲突
    ///
    /// # 参数
    /// * `local` - 本地文件信息（None 表示文件不存在）
    /// * `remote` - 远程文件信息
    ///
    /// # 返回
    /// * `Some(ConflictResolution)` - 冲突解决结果
    /// * `None` - 无需同步（版本相同）
    pub fn resolve(
        &self,
        local: Option<&FileInfo>,
        remote: &FileInfo,
    ) -> Option<ConflictResolution> {
        // 如果本地文件不存在，直接接受远程版本
        let local = match local {
            Some(l) => l,
            None => {
                debug!("本地文件不存在，接受远程版本: {}", remote.name);
                return Some(ConflictResolution::RemoteWins);
            }
        };

        // 比较版本向量
        match local.version.compare(&remote.version) {
            Some(Ordering::Equal) => {
                // 版本相同，无需同步
                debug!("版本相同，无需同步: {}", remote.name);
                None
            }
            Some(Ordering::Greater) => {
                // 本地版本更新
                debug!("本地版本更新，跳过: {}", remote.name);
                Some(ConflictResolution::LocalWins)
            }
            Some(Ordering::Less) => {
                // 远程版本更新
                debug!("远程版本更新，需要同步: {}", remote.name);
                Some(ConflictResolution::RemoteWins)
            }
            None => {
                // 检测到冲突（并发更新）
                warn!("检测到文件冲突: {}", remote.name);
                let conflict_path = self.generate_conflict_name(&remote.name);
                Some(ConflictResolution::Conflict {
                    conflict_copy_path: conflict_path,
                })
            }
        }
    }

    /// 生成冲突副本的文件名
    ///
    /// 格式: `filename.sync-conflict-YYYYMMDD-HHMMSS.ext`
    /// 例如: `document.txt` -> `document.sync-conflict-20240101-120000.txt`
    ///
    /// # 参数
    /// * `original_name` - 原始文件名
    ///
    /// # 返回
    /// 冲突副本的文件名
    pub fn generate_conflict_name(&self, original_name: &str) -> String {
        let now: DateTime<Local> = Local::now();
        let timestamp = now.format("%Y%m%d-%H%M%S").to_string();

        let path = Path::new(original_name);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(original_name);
        let extension = path.extension().and_then(|e| e.to_str());

        match extension {
            Some(ext) => format!("{}.sync-conflict-{}-{}.{}", stem, timestamp, self.local_device.short_id(), ext),
            None => format!("{}.sync-conflict-{}-{}", stem, timestamp, self.local_device.short_id()),
        }
    }

    /// 检查两个版本向量是否存在冲突
    pub fn detect_conflict(v1: &VersionVector, v2: &VersionVector) -> bool {
        v1.is_concurrent_with(v2)
    }

    /// 使用 "last writer wins" 策略解决冲突
    ///
    /// 当检测到冲突时，根据修改时间决定哪个版本胜出
    ///
    /// # 参数
    /// * `local` - 本地文件信息
    /// * `remote` - 远程文件信息
    ///
    /// # 返回
    /// true 表示本地版本胜出，false 表示远程版本胜出
    pub fn last_writer_wins(
        &self,
        local: &FileInfo,
        remote: &FileInfo,
        remote_device: syncthing_core::types::DeviceId,
    ) -> bool {
        match local.modified.cmp(&remote.modified) {
            Ordering::Greater => {
                info!(
                    "Last writer wins: 本地版本更新 (local: {:?}, remote: {:?})",
                    local.modified, remote.modified
                );
                true
            }
            Ordering::Less => {
                info!(
                    "Last writer wins: 远程版本更新 (local: {:?}, remote: {:?})",
                    local.modified, remote.modified
                );
                false
            }
            Ordering::Equal => {
                // 修改时间相同，使用设备 ID 作为决胜因素（确定性选择）
                let local_wins = self.local_device.as_bytes().as_slice() > remote_device.as_bytes().as_slice();
                info!(
                    "Last writer wins: 修改时间相同，使用设备ID决胜: {}",
                    if local_wins { "本地" } else { "远程" }
                );
                local_wins
            }
        }
    }

    /// 合并版本向量（用于冲突解决后）
    ///
    /// 当发生冲突并解决后，需要将两个版本向量合并，
    /// 以确保后续更新能正确识别这个冲突已解决
    pub fn merge_versions(&self, local: &FileInfo, remote: &FileInfo) -> VersionVector {
        local.version.merged(&remote.version)
    }
}

/// 冲突文件信息
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    /// 文件夹 ID
    pub folder: FolderId,
    /// 原始文件名
    pub original_name: String,
    /// 冲突副本文件名
    pub conflict_name: String,
    /// 本地版本
    pub local_version: VersionVector,
    /// 远程版本
    pub remote_version: VersionVector,
    /// 冲突时间
    pub conflict_time: SystemTime,
}

/// 冲突管理器
#[derive(Debug, Default)]
pub struct ConflictManager {
    /// 已记录的冲突
    conflicts: Vec<ConflictInfo>,
}

impl ConflictManager {
    /// 创建新的冲突管理器
    pub fn new() -> Self {
        Self {
            conflicts: Vec::new(),
        }
    }

    /// 记录一个冲突
    pub fn record_conflict(&mut self, info: ConflictInfo) {
        warn!(
            "记录文件冲突: folder={}, original={}, conflict={}",
            info.folder, info.original_name, info.conflict_name
        );
        self.conflicts.push(info);
    }

    /// 获取所有冲突
    pub fn get_conflicts(&self) -> &[ConflictInfo] {
        &self.conflicts
    }

    /// 获取特定文件夹的冲突
    pub fn get_folder_conflicts(&self, folder: &FolderId) -> Vec<&ConflictInfo> {
        self.conflicts
            .iter()
            .filter(|c| c.folder == *folder)
            .collect()
    }

    /// 清除已解决的冲突
    pub fn clear_resolved(&mut self, folder: &FolderId, file_name: &str) {
        self.conflicts
            .retain(|c| c.folder != *folder || c.original_name != file_name);
    }

    /// 获取冲突数量
    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::{DeviceId, FileInfo};

    fn create_test_device(id: u8) -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        DeviceId::from_bytes(bytes)
    }

    fn create_test_file(name: &str, version: VersionVector) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            size: 100,
            modified: SystemTime::now(),
            version,
            blocks: vec![],
            permissions: 0o644,
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            sequence: 1,
        }
    }

    #[test]
    fn test_conflict_resolution_remote_wins_when_local_missing() {
        let device = create_test_device(1);
        let resolver = ConflictResolver::new(device);

        let mut remote_version = VersionVector::new();
        remote_version.increment(device);
        let remote = create_test_file("test.txt", remote_version);

        let result = resolver.resolve(None, &remote);
        assert_eq!(result, Some(ConflictResolution::RemoteWins));
    }

    #[test]
    fn test_conflict_resolution_equal_versions() {
        let device = create_test_device(1);
        let resolver = ConflictResolver::new(device);

        let mut version = VersionVector::new();
        version.increment(device);

        let local = create_test_file("test.txt", version.clone());
        let remote = create_test_file("test.txt", version);

        let result = resolver.resolve(Some(&local), &remote);
        assert_eq!(result, None); // 无需同步
    }

    #[test]
    fn test_conflict_resolution_local_wins() {
        let device = create_test_device(1);
        let resolver = ConflictResolver::new(device);

        let mut local_version = VersionVector::new();
        local_version.increment(device);
        local_version.increment(device); // 版本 2

        let mut remote_version = VersionVector::new();
        remote_version.increment(device); // 版本 1

        let local = create_test_file("test.txt", local_version);
        let remote = create_test_file("test.txt", remote_version);

        let result = resolver.resolve(Some(&local), &remote);
        assert_eq!(result, Some(ConflictResolution::LocalWins));
    }

    #[test]
    fn test_conflict_resolution_remote_wins() {
        let device = create_test_device(1);
        let resolver = ConflictResolver::new(device);

        let mut local_version = VersionVector::new();
        local_version.increment(device); // 版本 1

        let mut remote_version = VersionVector::new();
        remote_version.increment(device);
        remote_version.increment(device); // 版本 2

        let local = create_test_file("test.txt", local_version);
        let remote = create_test_file("test.txt", remote_version);

        let result = resolver.resolve(Some(&local), &remote);
        assert_eq!(result, Some(ConflictResolution::RemoteWins));
    }

    #[test]
    fn test_conflict_detection() {
        let d1 = create_test_device(1);
        let d2 = create_test_device(2);
        let resolver = ConflictResolver::new(d1);

        let mut local_version = VersionVector::new();
        local_version.increment(d1); // d1 更新

        let mut remote_version = VersionVector::new();
        remote_version.increment(d2); // d2 更新（并发）

        let local = create_test_file("test.txt", local_version);
        let remote = create_test_file("test.txt", remote_version);

        let result = resolver.resolve(Some(&local), &remote);
        assert!(
            matches!(result, Some(ConflictResolution::Conflict { .. })),
            "应该检测到冲突"
        );
    }

    #[test]
    fn test_generate_conflict_name() {
        let device = create_test_device(1);
        let resolver = ConflictResolver::new(device);

        let conflict_name = resolver.generate_conflict_name("document.txt");
        assert!(conflict_name.contains(".sync-conflict-"));
        assert!(conflict_name.ends_with(".txt"));
        assert!(conflict_name.contains("document"));

        // 测试没有扩展名的文件
        let conflict_name_no_ext = resolver.generate_conflict_name("README");
        assert!(conflict_name_no_ext.contains(".sync-conflict-"));
        // 文件名中应该包含原始文件名和设备ID
        assert!(conflict_name_no_ext.starts_with("README"));
    }

    #[test]
    fn test_conflict_manager() {
        let mut manager = ConflictManager::new();
        let folder = FolderId::new("test-folder");

        let conflict = ConflictInfo {
            folder: folder.clone(),
            original_name: "file.txt".to_string(),
            conflict_name: "file.sync-conflict-20240101-120000.txt".to_string(),
            local_version: VersionVector::new(),
            remote_version: VersionVector::new(),
            conflict_time: SystemTime::now(),
        };

        manager.record_conflict(conflict);
        assert_eq!(manager.conflict_count(), 1);

        let folder_conflicts = manager.get_folder_conflicts(&folder);
        assert_eq!(folder_conflicts.len(), 1);

        manager.clear_resolved(&folder, "file.txt");
        assert_eq!(manager.conflict_count(), 0);
    }
}
