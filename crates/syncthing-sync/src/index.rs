//! 索引管理器
//!
//! 实现 Delta Index（增量索引）机制，支持全量索引和增量更新

use crate::database::LocalDatabase;
use crate::error::Result;
use std::collections::HashMap;
use std::sync::Arc;
use syncthing_core::types::{FileInfo, IndexID, IndexDelta};
use syncthing_core::DeviceId;
use tracing::{debug, info, warn};

/// 索引管理器
///
/// 负责维护本地索引状态、生成增量索引、以及与远程设备的索引同步
pub struct IndexManager {
    db: Arc<dyn LocalDatabase>,
    folder: String,
    local_index_id: Option<IndexID>,
    local_max_sequence: u64,
    remote_index_ids: HashMap<DeviceId, IndexID>,
    remote_max_sequences: HashMap<DeviceId, u64>,
}

impl IndexManager {
    /// 创建新的索引管理器
    pub async fn new(db: Arc<dyn LocalDatabase>, folder: impl Into<String>) -> Result<Self> {
        let folder = folder.into();

        // 尝试从数据库加载索引元数据
        let (local_index_id, local_max_sequence) = match db.get_folder_index_meta(&folder).await? {
            Some((id, seq)) => (Some(id), seq),
            None => (None, 0),
        };

        Ok(Self {
            db,
            folder,
            local_index_id,
            local_max_sequence,
            remote_index_ids: HashMap::new(),
            remote_max_sequences: HashMap::new(),
        })
    }

    /// 获取本地 IndexID
    pub fn local_index_id(&self) -> Option<IndexID> {
        self.local_index_id
    }

    /// 获取本地最大序列号
    pub fn local_max_sequence(&self) -> u64 {
        self.local_max_sequence
    }

    /// 获取远程设备的 IndexID
    pub fn remote_index_id(&self, device: &DeviceId) -> Option<IndexID> {
        self.remote_index_ids.get(device).copied()
    }

    /// 获取远程设备已知的最大序列号
    pub fn remote_max_sequence(&self, device: &DeviceId) -> Option<u64> {
        self.remote_max_sequences.get(device).copied()
    }

    /// 注册远程设备的索引状态
    pub fn register_remote_index(&mut self, device: DeviceId, index_id: IndexID, max_sequence: u64) {
        self.remote_index_ids.insert(device, index_id);
        self.remote_max_sequences.insert(device, max_sequence);
    }

    /// 获取指定设备的索引增量
    ///
    /// - 如果远程设备的 IndexID 与本地匹配，返回 sequence > remote_max_sequence 的文件
    /// - 如果不匹配或远程没有记录，返回 None（需要发送全量索引）
    pub async fn get_index_delta(&self, device: &DeviceId) -> Option<IndexDelta> {
        let local_index_id = self.local_index_id?;
        let remote_index_id = self.remote_index_ids.get(device)?;

        if local_index_id != *remote_index_id {
            warn!(
                folder = %self.folder,
                device = %device.short_id(),
                "IndexID mismatch, need full index"
            );
            return None;
        }

        let remote_sequence = self.remote_max_sequences.get(device).copied().unwrap_or(0);

        // 从数据库获取序列号大于 remote_sequence 的文件
        let files = match self.db.get_needed_files(&self.folder, remote_sequence).await {
            Ok(files) => files,
            Err(e) => {
                warn!(
                    folder = %self.folder,
                    device = %device.short_id(),
                    error = %e,
                    "Failed to get needed files for delta"
                );
                return None;
            }
        };

        debug!(
            folder = %self.folder,
            device = %device.short_id(),
            remote_sequence = remote_sequence,
            file_count = files.len(),
            "Generated index delta"
        );

        Some(IndexDelta {
            folder: self.folder.clone(),
            index_id: local_index_id,
            start_sequence: remote_sequence,
            files,
        })
    }

    /// 更新本地 IndexID（在重大变更时调用，如首次扫描、重建索引）
    ///
    /// 生成新的随机 IndexID 并将序列号重置为 0
    pub async fn update_index_id(&mut self) -> Result<IndexID> {
        let new_id = IndexID::random();
        self.local_index_id = Some(new_id);
        self.local_max_sequence = 0;

        // 持久化到数据库
        self.db
            .update_folder_index_meta(&self.folder, new_id, 0)
            .await?;

        info!(
            folder = %self.folder,
            index_id = ?new_id,
            "Updated local index ID"
        );

        Ok(new_id)
    }

    /// 更新索引（全量替换）
    ///
    /// 为每个文件分配递增的序列号，并持久化到数据库
    pub async fn update_index(&mut self, files: &mut [FileInfo]) -> Result<()> {
        // 确保有 IndexID（先获取，避免重置已分配的 sequence）
        let index_id = match self.local_index_id {
            Some(id) => id,
            None => {
                let id = IndexID::random();
                self.local_index_id = Some(id);
                id
            }
        };

        for file in files.iter_mut() {
            self.local_max_sequence += 1;
            file.sequence = self.local_max_sequence;
        }

        // 持久化文件和元数据
        self.db
            .update_files(&self.folder, files.to_vec())
            .await?;
        self.db
            .update_folder_index_meta(&self.folder, index_id, self.local_max_sequence)
            .await?;

        debug!(
            folder = %self.folder,
            file_count = files.len(),
            max_sequence = self.local_max_sequence,
            "Updated full index"
        );

        Ok(())
    }

    /// 更新索引增量
    ///
    /// 为每个文件分配递增的序列号，并持久化到数据库
    pub async fn update_index_delta(&mut self, files: &mut [FileInfo]) -> Result<()> {
        // 确保有 IndexID（先获取，避免重置已分配的 sequence）
        let index_id = match self.local_index_id {
            Some(id) => id,
            None => {
                let id = IndexID::random();
                self.local_index_id = Some(id);
                id
            }
        };

        for file in files.iter_mut() {
            self.local_max_sequence += 1;
            file.sequence = self.local_max_sequence;
        }

        // 持久化文件和元数据
        self.db
            .update_files(&self.folder, files.to_vec())
            .await?;
        self.db
            .update_folder_index_meta(&self.folder, index_id, self.local_max_sequence)
            .await?;

        debug!(
            folder = %self.folder,
            file_count = files.len(),
            max_sequence = self.local_max_sequence,
            "Updated index delta"
        );

        Ok(())
    }

    /// 决定发送全量索引还是增量更新
    ///
    /// 如果 delta 可用且非空，返回增量；否则返回全量文件列表
    pub async fn prepare_index_for_device(&self, device: &DeviceId) -> Result<EitherIndex> {
        match self.get_index_delta(device).await {
            Some(delta) if !delta.files.is_empty() => Ok(EitherIndex::Delta(delta)),
            _ => {
                let files = self.db.get_folder_files(&self.folder).await?;
                Ok(EitherIndex::Full(files))
            }
        }
    }
}

/// 索引发送类型：全量或增量
#[derive(Debug, Clone)]
pub enum EitherIndex {
    /// 全量索引
    Full(Vec<FileInfo>),
    /// 增量索引
    Delta(IndexDelta),
}

impl std::fmt::Debug for IndexManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexManager")
            .field("folder", &self.folder)
            .field("local_index_id", &self.local_index_id)
            .field("local_max_sequence", &self.local_max_sequence)
            .field("remote_index_ids", &self.remote_index_ids)
            .field("remote_max_sequences", &self.remote_max_sequences)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryDatabase;
    use syncthing_core::types::{FileType, Vector};

    fn create_test_file(name: &str) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            file_type: FileType::File,
            size: 100,
            permissions: 0o644,
            modified_s: 1234567890,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 0,
            block_size: 128 * 1024,
            blocks: vec![],
            symlink_target: None,
            deleted: None,
        }
    }

    #[tokio::test]
    async fn test_full_index_updates_index_id_and_sequence() {
        let db = MemoryDatabase::new();
        let mut manager = IndexManager::new(db, "test").await.unwrap();

        // 初始状态
        assert!(manager.local_index_id().is_none());
        assert_eq!(manager.local_max_sequence(), 0);

        let mut files = vec![create_test_file("a.txt"), create_test_file("b.txt")];
        manager.update_index(&mut files).await.unwrap();

        // 更新后应有 IndexID 和递增的 sequence
        assert!(manager.local_index_id().is_some());
        assert_eq!(manager.local_max_sequence(), 2);
        assert_eq!(files[0].sequence, 1);
        assert_eq!(files[1].sequence, 2);
    }

    #[tokio::test]
    async fn test_delta_returns_only_newer_files() {
        let db = MemoryDatabase::new();
        let mut manager = IndexManager::new(db.clone(), "test").await.unwrap();

        // 初始全量索引
        let mut files = vec![create_test_file("a.txt"), create_test_file("b.txt")];
        manager.update_index(&mut files).await.unwrap();

        let local_id = manager.local_index_id().unwrap();
        let device = DeviceId::random();

        // 注册远程设备已知状态：IndexID 匹配，序列号到 1
        manager.register_remote_index(device, local_id, 1);

        // 增量应只返回 sequence > 1 的文件
        let delta = manager.get_index_delta(&device).await;
        assert!(delta.is_some());
        let delta = delta.unwrap();
        assert_eq!(delta.files.len(), 1);
        assert_eq!(delta.files[0].name, "b.txt");
        assert_eq!(delta.files[0].sequence, 2);
    }

    #[tokio::test]
    async fn test_delta_returns_none_when_index_id_changes() {
        let db = MemoryDatabase::new();
        let mut manager = IndexManager::new(db, "test").await.unwrap();

        let mut files = vec![create_test_file("a.txt")];
        manager.update_index(&mut files).await.unwrap();

        let old_id = manager.local_index_id().unwrap();
        let device = DeviceId::random();
        manager.register_remote_index(device, old_id, 0);

        // 改变 IndexID
        manager.update_index_id().await.unwrap();

        // 增量应返回 None，触发全量重传
        let delta = manager.get_index_delta(&device).await;
        assert!(delta.is_none());
    }

    #[tokio::test]
    async fn test_delta_returns_none_for_unknown_device() {
        let db = MemoryDatabase::new();
        let mut manager = IndexManager::new(db, "test").await.unwrap();

        let mut files = vec![create_test_file("a.txt")];
        manager.update_index(&mut files).await.unwrap();

        let device = DeviceId::random();
        // 未注册远程设备

        let delta = manager.get_index_delta(&device).await;
        assert!(delta.is_none());
    }

    #[tokio::test]
    async fn test_index_delta_assigns_sequences() {
        let db = MemoryDatabase::new();
        let mut manager = IndexManager::new(db, "test").await.unwrap();

        let mut files = vec![create_test_file("a.txt")];
        manager.update_index_delta(&mut files).await.unwrap();

        assert_eq!(files[0].sequence, 1);
        assert_eq!(manager.local_max_sequence(), 1);

        let mut more_files = vec![create_test_file("b.txt")];
        manager.update_index_delta(&mut more_files).await.unwrap();
        assert_eq!(more_files[0].sequence, 2);
    }

    #[tokio::test]
    async fn test_persistence() {
        let db = MemoryDatabase::new();
        {
            let mut manager = IndexManager::new(db.clone(), "test").await.unwrap();
            let mut files = vec![create_test_file("a.txt")];
            manager.update_index(&mut files).await.unwrap();
        }

        // 重新创建 manager，应恢复之前的状态
        let manager = IndexManager::new(db, "test").await.unwrap();
        assert!(manager.local_index_id().is_some());
        assert_eq!(manager.local_max_sequence(), 1);
    }
}
