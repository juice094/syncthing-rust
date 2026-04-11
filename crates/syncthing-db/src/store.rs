//! Module: syncthing-db
//! Worker: Agent-E
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! BlockStore trait implementation
//!
//! This module provides the main implementation of the `BlockStore` trait
//! from syncthing-core, combining block storage, caching, and metadata management.

use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use syncthing_core::{
    traits::{BlockStore, FolderStats},
    BlockHash, FileInfo, FolderId, Result, SyncthingError,
};

use crate::kv::SledStore;
use crate::metadata::MetadataStore;

/// Key prefixes for different data types in the database
mod key_prefix {
    /// Device index: device/<device_id>/...
    pub const DEVICE: &[u8] = b"device/";
    /// Folder index: folder/<folder_id>/...
    pub const FOLDER: &[u8] = b"folder/";
    /// Block storage: block/<hash>
    pub const BLOCK: &[u8] = b"block/";
    /// Global index: global/<folder_id>/...
    pub const GLOBAL: &[u8] = b"global/";
    /// Metadata: meta/...
    pub const META: &[u8] = b"meta/";
}

/// Block store implementation
///
/// This is the main implementation of the `BlockStore` trait, providing:
/// - Content-addressed block storage
/// - File metadata and index management
/// - Folder statistics tracking
/// - Device-specific file tracking
#[derive(Debug)]
pub struct BlockStoreImpl {
    /// Underlying KV store
    store: SledStore,
    /// Metadata store for file indices
    metadata: MetadataStore,
    /// Cache for frequently accessed blocks
    block_cache: Arc<RwLock<lru::LruCache<BlockHash, Vec<u8>>>>,
}

impl BlockStoreImpl {
    /// Open a block store at the given path
    ///
    /// # Arguments
    /// * `path` - Path to the database directory
    ///
    /// # Returns
    /// A new `BlockStoreImpl` instance
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let store = SledStore::open(path)?;
        Self::from_store(store)
    }

    /// Create an in-memory block store
    ///
    /// Useful for testing and temporary storage.
    pub fn open_in_memory() -> Result<Self> {
        let store = SledStore::open_in_memory()?;
        Self::from_store(store)
    }

    /// Create a block store from an existing SledStore
    fn from_store(store: SledStore) -> Result<Self> {
        // Use the same store for metadata but with a separate tree
        let metadata_tree = store.open_tree("metadata")?;
        let metadata = MetadataStore::from_tree(metadata_tree);

        // Create LRU cache with default size (64MB worth of blocks)
        let cache = lru::LruCache::new(NonZero::new(1024).unwrap());

        Ok(Self {
            store,
            metadata,
            block_cache: Arc::new(RwLock::new(cache)),
        })
    }

    /// Create a block store with custom cache capacity
    ///
    /// # Arguments
    /// * `path` - Path to the database directory
    /// * `cache_capacity` - Maximum number of blocks to cache
    pub fn open_with_cache<P: AsRef<Path>>(path: P, cache_capacity: usize) -> Result<Self> {
        let store = SledStore::open(path)?;
        let metadata_tree = store.open_tree("metadata")?;
        let metadata = MetadataStore::from_tree(metadata_tree);
        let cache = lru::LruCache::new(NonZero::new(cache_capacity).unwrap());

        Ok(Self {
            store,
            metadata,
            block_cache: Arc::new(RwLock::new(cache)),
        })
    }

    /// Create a storage key for a block hash
    fn block_key(hash: &BlockHash) -> Vec<u8> {
        let mut key = key_prefix::BLOCK.to_vec();
        key.extend_from_slice(hash.as_bytes());
        key
    }

    /// Flush all pending writes to disk
    pub async fn flush(&self) -> Result<()> {
        self.store.flush()?;
        self.metadata.flush().await?;
        Ok(())
    }

    /// Get file info for a specific device
    ///
    /// # Arguments
    /// * `device_id` - The device ID
    /// * `folder` - The folder ID
    /// * `name` - The file name
    pub async fn get_device_file(
        &self,
        device_id: &str,
        folder: &FolderId,
        name: &str,
    ) -> Result<Option<FileInfo>> {
        self.metadata.get_device_file(device_id, folder, name).await
    }

    /// Store file info for a specific device
    ///
    /// # Arguments
    /// * `device_id` - The device ID
    /// * `folder` - The folder ID
    /// * `info` - The file info to store
    pub async fn put_device_file(
        &self,
        device_id: &str,
        folder: &FolderId,
        info: &FileInfo,
    ) -> Result<()> {
        self.metadata.put_device_file(device_id, folder, info).await
    }

    /// List all folders in the database
    pub async fn list_folders(&self) -> Result<Vec<FolderId>> {
        // Scan for folder stats keys to find all folders
        let results = self.store.scan_prefix(key_prefix::FOLDER)?;
        let mut folders = Vec::new();

        for (key, _) in results {
            // Extract folder ID from key
            if let Ok(key_str) = std::str::from_utf8(&key) {
                // Key format: folder/<folder_id>/...
                let parts: Vec<&str> = key_str.split('/').collect();
                if parts.len() >= 2 {
                    folders.push(FolderId::new(parts[1]));
                }
            }
        }

        // Deduplicate
        folders.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        folders.dedup_by(|a, b| a.as_str() == b.as_str());

        Ok(folders)
    }
}

#[async_trait]
impl BlockStore for BlockStoreImpl {
    async fn put(&self, hash: BlockHash, data: &[u8]) -> Result<()> {
        // Verify hash integrity
        let computed_hash = BlockHash::from_data(data);
        if computed_hash != hash {
            return Err(SyncthingError::protocol(format!(
                "Hash mismatch: expected {}, computed {}",
                hash, computed_hash
            )));
        }

        let key = Self::block_key(&hash);

        // Store in database
        self.store.put(&key, data).map_err(|e| {
            SyncthingError::Storage(format!("Failed to store block: {}", e))
        })?;

        // Update cache
        let mut cache = self.block_cache.write().await;
        cache.put(hash, data.to_vec());

        Ok(())
    }

    async fn get(&self, hash: BlockHash) -> Result<Option<Vec<u8>>> {
        // Check cache first
        {
            let cache = self.block_cache.read().await;
            if let Some(data) = cache.peek(&hash) {
                return Ok(Some(data.clone()));
            }
        }

        // Fetch from store
        let key = Self::block_key(&hash);
        match self.store.get(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to get block: {}", e))
        })? {
            Some(data) => {
                // Add to cache
                let mut cache = self.block_cache.write().await;
                cache.put(hash, data.clone());
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    async fn has(&self, hash: BlockHash) -> Result<bool> {
        // Check cache first
        {
            let cache = self.block_cache.read().await;
            if cache.peek(&hash).is_some() {
                return Ok(true);
            }
        }

        // Check store
        let key = Self::block_key(&hash);
        self.store.contains(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to check block: {}", e))
        })
    }

    async fn delete(&self, hash: BlockHash) -> Result<()> {
        let key = Self::block_key(&hash);

        // Remove from store
        self.store.delete(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to delete block: {}", e))
        })?;

        // Remove from cache
        let mut cache = self.block_cache.write().await;
        cache.pop(&hash);

        Ok(())
    }

    async fn get_index(&self, folder: &FolderId) -> Result<Vec<FileInfo>> {
        self.metadata.get_folder_index(folder).await
    }

    async fn update_index(&self, folder: &FolderId, files: Vec<FileInfo>) -> Result<()> {
        self.metadata.update_index(folder, files).await
    }

    async fn update_index_delta(&self, folder: &FolderId, files: Vec<FileInfo>) -> Result<()> {
        self.metadata.update_index_delta(folder, files).await
    }

    async fn folder_stats(&self, folder: &FolderId) -> Result<FolderStats> {
        let stats = self.metadata.get_folder_stats(folder).await?;
        Ok(FolderStats {
            file_count: stats.file_count,
            total_bytes: stats.total_bytes,
            block_count: stats.block_count,
        })
    }
}

/// Builder for BlockStoreImpl
pub struct BlockStoreImplBuilder {
    path: Option<std::path::PathBuf>,
    cache_capacity: usize,
    in_memory: bool,
}

impl Default for BlockStoreImplBuilder {
    fn default() -> Self {
        Self {
            path: None,
            cache_capacity: 1024,
            in_memory: false,
        }
    }
}

impl BlockStoreImplBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the database path
    pub fn path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.path = Some(path.as_ref().to_path_buf());
        self.in_memory = false;
        self
    }

    /// Use in-memory storage
    pub fn in_memory(mut self) -> Self {
        self.in_memory = true;
        self.path = None;
        self
    }

    /// Set the cache capacity (number of blocks)
    pub fn cache_capacity(mut self, capacity: usize) -> Self {
        self.cache_capacity = capacity;
        self
    }

    /// Build the block store
    pub fn build(self) -> Result<BlockStoreImpl> {
        if self.in_memory {
            BlockStoreImpl::open_in_memory()
        } else if let Some(path) = self.path {
            BlockStoreImpl::open_with_cache(path, self.cache_capacity)
        } else {
            Err(SyncthingError::config(
                "Either path or in_memory must be specified".to_string()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_block_store_impl_basic() {
        let temp_dir = TempDir::new().unwrap();
        let store = BlockStoreImpl::open(temp_dir.path()).unwrap();

        let data = b"hello world";
        let hash = BlockHash::from_data(data);

        // Store and retrieve
        store.put(hash, data).await.unwrap();
        let retrieved = store.get(hash).await.unwrap();
        assert_eq!(retrieved, Some(data.to_vec()));

        // Has check
        assert!(store.has(hash).await.unwrap());

        // Delete
        store.delete(hash).await.unwrap();
        assert!(!store.has(hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_block_store_impl_in_memory() {
        let store = BlockStoreImpl::open_in_memory().unwrap();

        let data = b"in memory data";
        let hash = BlockHash::from_data(data);

        store.put(hash, data).await.unwrap();
        let retrieved = store.get(hash).await.unwrap();
        assert_eq!(retrieved, Some(data.to_vec()));
    }

    #[tokio::test]
    async fn test_block_store_impl_index() {
        let temp_dir = TempDir::new().unwrap();
        let store = BlockStoreImpl::open(temp_dir.path()).unwrap();

        let folder = FolderId::new("test-folder");
        let files = vec![
            FileInfo::new("file1.txt"),
            FileInfo::new("file2.txt"),
        ];

        store.update_index(&folder, files.clone()).await.unwrap();

        let index = store.get_index(&folder).await.unwrap();
        assert_eq!(index.len(), 2);
    }

    #[tokio::test]
    async fn test_block_store_impl_folder_stats() {
        let temp_dir = TempDir::new().unwrap();
        let store = BlockStoreImpl::open(temp_dir.path()).unwrap();

        let folder = FolderId::new("test-folder");

        let stats = store.folder_stats(&folder).await.unwrap();
        assert_eq!(stats.file_count, 0);

        let mut file = FileInfo::new("test.txt");
        file.size = 1000;
        store.update_index(&folder, vec![file]).await.unwrap();

        let stats = store.folder_stats(&folder).await.unwrap();
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.total_bytes, 1000);
    }

    #[tokio::test]
    async fn test_block_store_impl_hash_verification() {
        let store = BlockStoreImpl::open_in_memory().unwrap();

        let data = b"test data";
        let wrong_hash = BlockHash::from_bytes([0u8; 32]);

        let result = store.put(wrong_hash, data).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_builder_pattern() {
        let temp_dir = TempDir::new().unwrap();

        let store = BlockStoreImplBuilder::new()
            .path(temp_dir.path())
            .cache_capacity(512)
            .build()
            .unwrap();

        let data = b"builder test";
        let hash = BlockHash::from_data(data);
        store.put(hash, data).await.unwrap();

        assert!(store.has(hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_builder_in_memory() {
        let store = BlockStoreImplBuilder::new()
            .in_memory()
            .cache_capacity(256)
            .build()
            .unwrap();

        let data = b"in memory builder";
        let hash = BlockHash::from_data(data);
        store.put(hash, data).await.unwrap();

        assert!(store.has(hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_device_file_operations() {
        let temp_dir = TempDir::new().unwrap();
        let store = BlockStoreImpl::open(temp_dir.path()).unwrap();

        let folder = FolderId::new("test-folder");
        let device_id = "test-device-123";

        let file = FileInfo::new("device_file.txt");
        store.put_device_file(device_id, &folder, &file).await.unwrap();

        let retrieved = store.get_device_file(device_id, &folder, "device_file.txt").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "device_file.txt");
    }
}
