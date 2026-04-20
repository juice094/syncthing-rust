//! Module: syncthing-db
//! Worker: Agent-E
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Metadata storage for files and folders
//!
//! This module provides storage for file metadata (FileInfo) and
//! folder indices using sled as the underlying storage engine.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use syncthing_core::{BlockHash, FileInfo, FolderId, Result, SyncthingError};

use crate::kv::{SledStore, SledTree};

/// Metadata store for file and folder information
///
/// Stores file metadata (FileInfo) organized by folder.
/// Each folder has its own sled tree for isolation.
#[derive(Debug)]
pub struct MetadataStore {
    inner: StoreInner,
}

#[derive(Debug)]
enum StoreInner {
    Db(SledStore),
    Tree(SledTree),
}

impl StoreInner {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        match self {
            StoreInner::Db(store) => store.get(key),
            StoreInner::Tree(tree) => tree.get(key),
        }
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        match self {
            StoreInner::Db(store) => store.put(key, value),
            StoreInner::Tree(tree) => tree.put(key, value),
        }
    }

    fn delete(&self, key: &[u8]) -> Result<bool> {
        match self {
            StoreInner::Db(store) => store.delete(key),
            StoreInner::Tree(tree) => tree.delete(key),
        }
    }

    fn contains(&self, key: &[u8]) -> Result<bool> {
        match self {
            StoreInner::Db(store) => store.contains(key),
            StoreInner::Tree(tree) => tree.contains(key),
        }
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        match self {
            StoreInner::Db(store) => store.scan_prefix(prefix),
            StoreInner::Tree(tree) => tree.scan_prefix(prefix),
        }
    }

    fn apply_batch(&self, batch: Vec<(Vec<u8>, Option<Vec<u8>>)>) -> Result<()> {
        match self {
            StoreInner::Db(store) => store.apply_batch(batch),
            StoreInner::Tree(tree) => tree.apply_batch(batch),
        }
    }

    fn flush(&self) -> Result<()> {
        match self {
            StoreInner::Db(store) => store.flush(),
            StoreInner::Tree(tree) => tree.flush(),
        }
    }
}

/// Folder statistics summary
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderStats {
    /// Number of files in the folder
    pub file_count: u64,
    /// Total size of all files in bytes
    pub total_bytes: u64,
    /// Number of blocks stored for this folder
    pub block_count: u64,
}

/// Key for storing file info in a folder
///
/// Format: folder_id + ":" + file_name
fn make_file_key(folder: &FolderId, name: &str) -> Vec<u8> {
    let mut key = folder.as_str().as_bytes().to_vec();
    key.push(b':');
    key.extend_from_slice(name.as_bytes());
    key
}

/// Key for folder metadata
fn make_folder_stats_key(folder: &FolderId) -> Vec<u8> {
    let mut key = b"folder_stats:".to_vec();
    key.extend_from_slice(folder.as_str().as_bytes());
    key
}

/// Key prefix for folder index entries
fn make_folder_prefix(folder: &FolderId) -> Vec<u8> {
    let mut key = folder.as_str().as_bytes().to_vec();
    key.push(b':');
    key
}

/// Key for device index
/// Format: device/<device_id>/folder/<folder_id>/file/<file_name>
fn make_device_file_key(device_id: &str, folder: &FolderId, name: &str) -> Vec<u8> {
    format!("device/{}/folder/{}/file/{}", device_id, folder.as_str(), name)
        .into_bytes()
}

/// Key for block index
/// Format: block/<hash>
pub fn make_block_key(hash: &BlockHash) -> Vec<u8> {
    let mut key = b"block/".to_vec();
    key.extend_from_slice(hash.as_bytes());
    key
}

impl MetadataStore {
    /// Create a new metadata store
    ///
    /// # Arguments
    /// * `store` - The underlying sled store
    pub fn new(store: SledStore) -> Self {
        Self { inner: StoreInner::Db(store) }
    }

    /// Create a metadata store from an existing sled tree
    ///
    /// # Arguments
    /// * `tree` - The sled tree to use
    pub fn from_tree(tree: SledTree) -> Self {
        Self { inner: StoreInner::Tree(tree) }
    }

    /// Store file info
    ///
    /// # Arguments
    /// * `folder` - The folder ID
    /// * `info` - The file info to store
    pub async fn put_file(&self, folder: &FolderId, info: &FileInfo) -> Result<()> {
        let key = make_file_key(folder, &info.name);
        let value = serde_json::to_vec(info).map_err(|e| {
            SyncthingError::Storage(format!("Failed to serialize FileInfo: {}", e))
        })?;

        self.inner.put(&key, &value).map_err(|e| {
            SyncthingError::Storage(format!("Failed to store file info: {}", e))
        })?;

        // Update folder stats
        self.update_folder_stats(folder).await?;

        Ok(())
    }

    /// Get file info
    ///
    /// # Arguments
    /// * `folder` - The folder ID
    /// * `name` - The file name
    ///
    /// # Returns
    /// `Ok(Some(FileInfo))` if found, `Ok(None)` if not
    pub async fn get_file(&self, folder: &FolderId, name: &str) -> Result<Option<FileInfo>> {
        let key = make_file_key(folder, name);
        let value = self.inner.get(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to get file info: {}", e))
        })?;

        match value {
            Some(bytes) => {
                let info: FileInfo = serde_json::from_slice(&bytes).map_err(|e| {
                    SyncthingError::Storage(format!("Failed to deserialize FileInfo: {}", e))
                })?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }

    /// Get all files in folder
    ///
    /// # Arguments
    /// * `folder` - The folder ID
    ///
    /// # Returns
    /// Vector of all FileInfo in the folder
    pub async fn get_folder_index(&self, folder: &FolderId) -> Result<Vec<FileInfo>> {
        let prefix = make_folder_prefix(folder);
        let results = self.inner.scan_prefix(&prefix).map_err(|e| {
            SyncthingError::Storage(format!("Failed to scan folder index: {}", e))
        })?;

        let mut files = Vec::new();
        for (_, value) in results {
            let info: FileInfo = serde_json::from_slice(&value).map_err(|e| {
                SyncthingError::Storage(format!("Failed to deserialize FileInfo: {}", e))
            })?;
            files.push(info);
        }

        // Sort by sequence number for consistent ordering
        files.sort_by_key(|f| f.sequence);

        Ok(files)
    }

    /// Update the entire index for a folder
    ///
    /// Replaces all existing file entries for the folder.
    ///
    /// # Arguments
    /// * `folder` - The folder ID
    /// * `files` - New file index
    pub async fn update_index(&self, folder: &FolderId, files: Vec<FileInfo>) -> Result<()> {
        // First, delete all existing entries for this folder
        let prefix = make_folder_prefix(folder);
        let existing = self.inner.scan_prefix(&prefix).map_err(|e| {
            SyncthingError::Storage(format!("Failed to scan existing index: {}", e))
        })?;

        let mut batch: Vec<(Vec<u8>, Option<Vec<u8>>)> = existing
            .into_iter()
            .map(|(key, _)| (key, None))
            .collect();

        // Add all new entries
        for file in files {
            let key = make_file_key(folder, &file.name);
            let value = serde_json::to_vec(&file).map_err(|e| {
                SyncthingError::Storage(format!("Failed to serialize FileInfo: {}", e))
            })?;
            batch.push((key, Some(value)));
        }

        self.inner.apply_batch(batch).map_err(|e| {
            SyncthingError::Storage(format!("Failed to update index: {}", e))
        })?;

        // Update folder stats
        self.update_folder_stats(folder).await?;

        Ok(())
    }

    /// Update index with delta (partial update)
    ///
    /// Updates only the specified files, leaving others unchanged.
    ///
    /// # Arguments
    /// * `folder` - The folder ID
    /// * `files` - Files to update (or delete if marked as deleted)
    pub async fn update_index_delta(
        &self,
        folder: &FolderId,
        files: Vec<FileInfo>,
    ) -> Result<()> {
        let mut batch: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();

        for file in files {
            let key = make_file_key(folder, &file.name);

            // Always serialize and store, even for deleted files
            // Deleted files are kept in the index for sync purposes
            let value = serde_json::to_vec(&file).map_err(|e| {
                SyncthingError::Storage(format!("Failed to serialize FileInfo: {}", e))
            })?;
            batch.push((key, Some(value)));
        }

        self.inner.apply_batch(batch).map_err(|e| {
            SyncthingError::Storage(format!("Failed to apply index delta: {}", e))
        })?;

        // Update folder stats
        self.update_folder_stats(folder).await?;

        Ok(())
    }

    /// Delete a file from the index
    ///
    /// # Arguments
    /// * `folder` - The folder ID
    /// * `name` - The file name
    pub async fn delete_file(&self, folder: &FolderId, name: &str) -> Result<()> {
        let key = make_file_key(folder, name);
        self.inner.delete(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to delete file: {}", e))
        })?;

        // Update folder stats
        self.update_folder_stats(folder).await?;

        Ok(())
    }

    /// Get folder statistics
    ///
    /// # Arguments
    /// * `folder` - The folder ID
    pub async fn get_folder_stats(&self, folder: &FolderId) -> Result<FolderStats> {
        let key = make_folder_stats_key(folder);
        let value = self.inner.get(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to get folder stats: {}", e))
        })?;

        match value {
            Some(bytes) => {
                let stats: FolderStats = serde_json::from_slice(&bytes).map_err(|e| {
                    SyncthingError::Storage(format!("Failed to deserialize stats: {}", e))
                })?;
                Ok(stats)
            }
            None => Ok(FolderStats::default()),
        }
    }

    /// Recalculate and update folder statistics
    async fn update_folder_stats(&self, folder: &FolderId) -> Result<()> {
        let files = self.get_folder_index(folder).await?;

        let mut stats = FolderStats::default();
        let mut unique_blocks: HashMap<BlockHash, ()> = HashMap::new();

        for file in files {
            if !file.is_deleted() {
                stats.file_count += 1;
                stats.total_bytes += file.size as u64;
                for block in &file.blocks {
                    let hash_bytes: [u8; 32] = block.hash.as_slice().try_into().unwrap_or([0u8; 32]);
                    unique_blocks.insert(BlockHash::from_bytes(hash_bytes), ());
                }
            }
        }

        stats.block_count = unique_blocks.len() as u64;

        let key = make_folder_stats_key(folder);
        let value = serde_json::to_vec(&stats).map_err(|e| {
            SyncthingError::Storage(format!("Failed to serialize stats: {}", e))
        })?;

        self.inner.put(&key, &value).map_err(|e| {
            SyncthingError::Storage(format!("Failed to store folder stats: {}", e))
        })?;

        Ok(())
    }

    /// Check if a file exists in the index
    pub async fn file_exists(&self, folder: &FolderId, name: &str) -> Result<bool> {
        let key = make_file_key(folder, name);
        self.inner.contains(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to check file existence: {}", e))
        })
    }

    /// Get the number of files in a folder
    pub async fn file_count(&self, folder: &FolderId) -> Result<usize> {
        let prefix = make_folder_prefix(folder);
        let results = self.inner.scan_prefix(&prefix).map_err(|e| {
            SyncthingError::Storage(format!("Failed to count files: {}", e))
        })?;
        Ok(results.len())
    }

    /// Flush all pending changes to disk
    pub async fn flush(&self) -> Result<()> {
        self.inner.flush().map_err(|e| {
            SyncthingError::Storage(format!("Failed to flush metadata store: {}", e))
        })
    }

    /// Store file info for a specific device
    ///
    /// # Arguments
    /// * `device_id` - The device ID string
    /// * `folder` - The folder ID
    /// * `info` - The file info to store
    pub async fn put_device_file(
        &self,
        device_id: &str,
        folder: &FolderId,
        info: &FileInfo,
    ) -> Result<()> {
        let key = make_device_file_key(device_id, folder, &info.name);
        let value = serde_json::to_vec(info).map_err(|e| {
            SyncthingError::Storage(format!("Failed to serialize FileInfo: {}", e))
        })?;

        self.inner.put(&key, &value).map_err(|e| {
            SyncthingError::Storage(format!("Failed to store device file info: {}", e))
        })?;

        Ok(())
    }

    /// Get file info for a specific device
    ///
    /// # Arguments
    /// * `device_id` - The device ID string
    /// * `folder` - The folder ID
    /// * `name` - The file name
    pub async fn get_device_file(
        &self,
        device_id: &str,
        folder: &FolderId,
        name: &str,
    ) -> Result<Option<FileInfo>> {
        let key = make_device_file_key(device_id, folder, name);
        let value = self.inner.get(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to get device file info: {}", e))
        })?;

        match value {
            Some(bytes) => {
                let info: FileInfo = serde_json::from_slice(&bytes).map_err(|e| {
                    SyncthingError::Storage(format!("Failed to deserialize FileInfo: {}", e))
                })?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }

    /// Get all files for a device in a folder
    pub async fn get_device_files(
        &self,
        device_id: &str,
        folder: &FolderId,
    ) -> Result<Vec<FileInfo>> {
        let prefix = format!("device/{}/folder/{}/", device_id, folder.as_str());
        let results = self.inner.scan_prefix(prefix.as_bytes()).map_err(|e| {
            SyncthingError::Storage(format!("Failed to scan device files: {}", e))
        })?;

        let mut files = Vec::new();
        for (_, value) in results {
            let info: FileInfo = serde_json::from_slice(&value).map_err(|e| {
                SyncthingError::Storage(format!("Failed to deserialize FileInfo: {}", e))
            })?;
            files.push(info);
        }

        files.sort_by_key(|f| f.sequence);
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    use tempfile::TempDir;

    fn create_test_file_info(name: &str, size: u64) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            file_type: syncthing_core::FileType::File,
            size: size as i64,
            permissions: 0o644,
            modified_s: 0,
            modified_ns: 0,
            version: syncthing_core::Vector::new(),
            sequence: 0,
            block_size: 0,
            blocks: vec![],
            symlink_target: None,
            deleted: Some(false),
        }
    }

    #[tokio::test]
    async fn test_metadata_store_basic() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = MetadataStore::new(sled_store);

        let folder = FolderId::new("test-folder");
        let info = create_test_file_info("test.txt", 1024);

        // Test put and get
        store.put_file(&folder, &info).await.unwrap();
        let retrieved = store.get_file(&folder, "test.txt").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test.txt");

        // Test non-existent file
        let missing = store.get_file(&folder, "missing.txt").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_folder_index() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = MetadataStore::new(sled_store);

        let folder = FolderId::new("test-folder");

        // Add multiple files
        store.put_file(&folder, &create_test_file_info("file1.txt", 100)).await.unwrap();
        store.put_file(&folder, &create_test_file_info("file2.txt", 200)).await.unwrap();
        store.put_file(&folder, &create_test_file_info("file3.txt", 300)).await.unwrap();

        // Get folder index
        let index = store.get_folder_index(&folder).await.unwrap();
        assert_eq!(index.len(), 3);

        // Test file count
        let count = store.file_count(&folder).await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_update_index() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = MetadataStore::new(sled_store);

        let folder = FolderId::new("test-folder");

        // Add initial files
        store.put_file(&folder, &create_test_file_info("file1.txt", 100)).await.unwrap();
        store.put_file(&folder, &create_test_file_info("file2.txt", 200)).await.unwrap();

        // Replace with new index
        let new_index = vec![
            create_test_file_info("new1.txt", 1000),
            create_test_file_info("new2.txt", 2000),
            create_test_file_info("new3.txt", 3000),
        ];

        store.update_index(&folder, new_index).await.unwrap();

        let index = store.get_folder_index(&folder).await.unwrap();
        assert_eq!(index.len(), 3);

        // Old files should be gone
        assert!(store.get_file(&folder, "file1.txt").await.unwrap().is_none());
        assert!(store.get_file(&folder, "file2.txt").await.unwrap().is_none());

        // New files should exist
        assert!(store.get_file(&folder, "new1.txt").await.unwrap().is_some());
        assert!(store.get_file(&folder, "new2.txt").await.unwrap().is_some());
        assert!(store.get_file(&folder, "new3.txt").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_update_index_delta() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = MetadataStore::new(sled_store);

        let folder = FolderId::new("test-folder");

        // Add initial files
        store.put_file(&folder, &create_test_file_info("file1.txt", 100)).await.unwrap();
        store.put_file(&folder, &create_test_file_info("file2.txt", 200)).await.unwrap();

        // Update with delta
        let delta = vec![
            create_test_file_info("file2.txt", 250),  // Update
            create_test_file_info("file3.txt", 300),  // Add
        ];

        store.update_index_delta(&folder, delta).await.unwrap();

        let index = store.get_folder_index(&folder).await.unwrap();
        assert_eq!(index.len(), 3);

        // file1.txt should still exist
        assert!(store.get_file(&folder, "file1.txt").await.unwrap().is_some());

        // file2.txt should be updated
        let file2 = store.get_file(&folder, "file2.txt").await.unwrap().unwrap();
        assert_eq!(file2.size, 250);

        // file3.txt should be added
        assert!(store.get_file(&folder, "file3.txt").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_deleted_file() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = MetadataStore::new(sled_store);

        let folder = FolderId::new("test-folder");

        // Add a file
        let mut info = create_test_file_info("deleted.txt", 100);
        store.put_file(&folder, &info).await.unwrap();

        // Mark as deleted and update
        info.mark_deleted();
        store.update_index_delta(&folder, vec![info]).await.unwrap();

        // The file should still be in the index (for sync purposes)
        // but marked as deleted
        let retrieved = store.get_file(&folder, "deleted.txt").await.unwrap();
        assert!(retrieved.is_some());
        assert!(retrieved.unwrap().is_deleted());
    }

    #[tokio::test]
    async fn test_folder_stats() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = MetadataStore::new(sled_store);

        let folder = FolderId::new("test-folder");

        // Initially empty
        let stats = store.get_folder_stats(&folder).await.unwrap();
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.total_bytes, 0);

        // Add files
        store.put_file(&folder, &create_test_file_info("file1.txt", 100)).await.unwrap();
        store.put_file(&folder, &create_test_file_info("file2.txt", 200)).await.unwrap();

        // Stats should be updated
        let stats = store.get_folder_stats(&folder).await.unwrap();
        assert_eq!(stats.file_count, 2);
        assert_eq!(stats.total_bytes, 300);
    }

    #[tokio::test]
    async fn test_multiple_folders() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = MetadataStore::new(sled_store);

        let folder1 = FolderId::new("folder1");
        let folder2 = FolderId::new("folder2");

        store.put_file(&folder1, &create_test_file_info("file.txt", 100)).await.unwrap();
        store.put_file(&folder2, &create_test_file_info("file.txt", 200)).await.unwrap();

        // Files should be isolated by folder
        let file1 = store.get_file(&folder1, "file.txt").await.unwrap().unwrap();
        let file2 = store.get_file(&folder2, "file.txt").await.unwrap().unwrap();

        assert_eq!(file1.size, 100);
        assert_eq!(file2.size, 200);
    }

    #[tokio::test]
    async fn test_device_files() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = MetadataStore::new(sled_store);

        let folder = FolderId::new("test-folder");
        let device_id = "ABC123";

        // Store file for device
        let info = create_test_file_info("device_file.txt", 500);
        store.put_device_file(device_id, &folder, &info).await.unwrap();

        // Retrieve file for device
        let retrieved = store.get_device_file(device_id, &folder, "device_file.txt").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().size, 500);

        // Different device should not see the file
        let other = store.get_device_file("OTHER", &folder, "device_file.txt").await.unwrap();
        assert!(other.is_none());
    }
}
