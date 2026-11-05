//! Index manager.
//!
//! Implements the Delta Index (incremental index) mechanism, supporting both
//! full index and incremental updates.
//!
//! See "docs/design/ADR-20261105-index-sequence-authority.md".

use crate::database::LocalDatabase;
use crate::error::Result;
use std::collections::HashMap;
use std::sync::Arc;
use syncthing_core::types::{FileInfo, IndexDelta, IndexID};
use syncthing_core::DeviceId;
use tracing::{debug, info, warn};

/// Index manager for one folder.
///
/// Maintains local index state, generates index deltas, and tracks which
/// index state each remote device has received.
pub struct IndexManager {
    db: Arc<dyn LocalDatabase>,
    folder: String,
    local_index_id: Option<IndexID>,
    local_max_sequence: u64,
    remote_index_ids: HashMap<DeviceId, IndexID>,
    remote_max_sequences: HashMap<DeviceId, u64>,
}

impl IndexManager {
    /// Creates a new index manager, restoring index metadata from the database.
    ///
    /// The database is the single source of truth for sequence numbers (see
    /// the module-level ADR pointer).
    pub async fn new(db: Arc<dyn LocalDatabase>, folder: impl Into<String>) -> Result<Self> {
        let folder = folder.into();

        let local_index_id = db.get_folder_index_meta(&folder).await?.map(|(id, _)| id);
        let local_max_sequence = db.get_sequence(&folder).await?;

        Ok(Self {
            db,
            folder,
            local_index_id,
            local_max_sequence,
            remote_index_ids: HashMap::new(),
            remote_max_sequences: HashMap::new(),
        })
    }

    /// Returns the local index ID, if one has been assigned.
    pub fn local_index_id(&self) -> Option<IndexID> {
        self.local_index_id
    }

    /// Returns the local maximum sequence number.
    pub fn local_max_sequence(&self) -> u64 {
        self.local_max_sequence
    }

    /// Returns the index ID a remote device last acknowledged.
    pub fn remote_index_id(&self, device: &DeviceId) -> Option<IndexID> {
        self.remote_index_ids.get(device).copied()
    }

    /// Returns the maximum sequence number a remote device has received.
    pub fn remote_max_sequence(&self, device: &DeviceId) -> Option<u64> {
        self.remote_max_sequences.get(device).copied()
    }

    /// Records a remote device's acknowledged index state.
    pub fn register_remote_index(
        &mut self,
        device: DeviceId,
        index_id: IndexID,
        max_sequence: u64,
    ) {
        self.remote_index_ids.insert(device, index_id);
        self.remote_max_sequences.insert(device, max_sequence);
    }

    /// Returns the index delta for the given device, if one can be sent.
    ///
    /// - When the remote device's index ID matches the local one, returns the
    ///   files with sequence greater than the remote maximum.
    /// - When the IDs mismatch or the device is unknown, returns `None`
    ///   (a full index must be sent instead).
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

        let files = match self
            .db
            .get_needed_files(&self.folder, remote_sequence)
            .await
        {
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

    /// Rotates the local index ID (called on major changes such as the first
    /// scan or an index rebuild).
    ///
    /// Generates a new random index ID and resets the sequence number to 0.
    /// The database remains the single source of truth for sequences (see
    /// the module-level ADR pointer).
    pub async fn update_index_id(&mut self) -> Result<IndexID> {
        let new_id = IndexID::random();
        self.local_index_id = Some(new_id);
        self.local_max_sequence = 0;

        self.db.reset_sequence(&self.folder).await?;
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

    /// Replaces the full index, assigning increasing sequence numbers.
    ///
    /// Sequences are allocated from the database (the single source of
    /// truth) and all files plus metadata are persisted.
    pub async fn update_index(&mut self, files: &mut [FileInfo]) -> Result<()> {
        // Fetch (or create) the index ID before assigning sequences, so the
        // sequence reset implied by a fresh ID never discards already
        // allocated sequence numbers.
        let index_id = match self.local_index_id {
            Some(id) => id,
            None => {
                let id = IndexID::random();
                self.local_index_id = Some(id);
                id
            }
        };

        for file in files.iter_mut() {
            file.sequence = self.db.increment_sequence(&self.folder).await?;
        }

        self.db.update_files(&self.folder, files.to_vec()).await?;
        let max_sequence = self.db.get_sequence(&self.folder).await?;
        self.local_max_sequence = max_sequence;
        self.db
            .update_folder_index_meta(&self.folder, index_id, max_sequence)
            .await?;

        debug!(
            folder = %self.folder,
            file_count = files.len(),
            max_sequence,
            "Updated full index"
        );

        Ok(())
    }

    /// Applies an incremental index update, assigning increasing sequence numbers.
    ///
    /// Sequences are allocated from the database (the single source of
    /// truth) and all files plus metadata are persisted.
    pub async fn update_index_delta(&mut self, files: &mut [FileInfo]) -> Result<()> {
        // Fetch (or create) the index ID before assigning sequences, so the
        // sequence reset implied by a fresh ID never discards already
        // allocated sequence numbers.
        let index_id = match self.local_index_id {
            Some(id) => id,
            None => {
                let id = IndexID::random();
                self.local_index_id = Some(id);
                id
            }
        };

        for file in files.iter_mut() {
            file.sequence = self.db.increment_sequence(&self.folder).await?;
        }

        self.db.update_files(&self.folder, files.to_vec()).await?;
        let max_sequence = self.db.get_sequence(&self.folder).await?;
        self.local_max_sequence = max_sequence;
        self.db
            .update_folder_index_meta(&self.folder, index_id, max_sequence)
            .await?;

        debug!(
            folder = %self.folder,
            file_count = files.len(),
            max_sequence,
            "Updated index delta"
        );

        Ok(())
    }

    /// Decides between sending a full index or an incremental update.
    ///
    /// Returns the delta when one is available and non-empty; otherwise
    /// returns the full file list.
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

/// Index payload to send: full or delta.
#[derive(Debug, Clone)]
pub enum EitherIndex {
    /// Full index (complete file list).
    Full(Vec<FileInfo>),
    /// Incremental index delta.
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
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
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

    #[tokio::test]
    async fn test_sequence_shared_with_scanner_path() {
        let db = MemoryDatabase::new();

        // 模拟 Scanner 先分配了两个 sequence
        let scanner_seq_1 = db.increment_sequence("test").await.unwrap();
        let scanner_seq_2 = db.increment_sequence("test").await.unwrap();
        assert_eq!(scanner_seq_1, 1);
        assert_eq!(scanner_seq_2, 2);

        // IndexManager 从 db 恢复 max sequence
        let mut manager = IndexManager::new(db.clone(), "test").await.unwrap();
        assert_eq!(manager.local_max_sequence(), 2);

        // IndexManager 继续分配 sequence，应与 Scanner 路径连续
        let mut files = vec![create_test_file("c.txt")];
        manager.update_index_delta(&mut files).await.unwrap();
        assert_eq!(files[0].sequence, 3);
        assert_eq!(manager.local_max_sequence(), 3);

        // 重启后仍保持单调
        drop(manager);
        let mut manager = IndexManager::new(db, "test").await.unwrap();
        assert_eq!(manager.local_max_sequence(), 3);

        let mut more_files = vec![create_test_file("d.txt")];
        manager.update_index_delta(&mut more_files).await.unwrap();
        assert_eq!(more_files[0].sequence, 4);
    }
}
