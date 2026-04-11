//! Module: syncthing-db
//! Worker: Agent-E
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Block storage and caching
//!
//! This module provides a content-addressed block store with LRU caching.
//! Implements the `BlockStore` trait from syncthing-core.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use syncthing_core::{
    traits::{BlockStore, FolderStats},
    BlockHash, FileInfo, FolderId, Result, SyncthingError,
};

use crate::kv::SledStore;
use crate::metadata::MetadataStore;

/// Cache entry with access tracking
#[derive(Debug, Clone)]
struct CacheEntry {
    data: Vec<u8>,
    last_access: std::time::Instant,
    access_count: u64,
}

impl CacheEntry {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            last_access: std::time::Instant::now(),
            access_count: 1,
        }
    }

    fn touch(&mut self) {
        self.last_access = std::time::Instant::now();
        self.access_count += 1;
    }
}

/// LRU cache for frequently accessed blocks
#[derive(Debug)]
struct LruCache {
    entries: HashMap<BlockHash, CacheEntry>,
    max_size: usize,
    current_size: usize,
}

impl LruCache {
    fn new(max_size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_size,
            current_size: 0,
        }
    }

    fn get(&mut self, hash: &BlockHash) -> Option<Vec<u8>> {
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.touch();
            Some(entry.data.clone())
        } else {
            None
        }
    }

    fn peek(&self, hash: &BlockHash) -> Option<Vec<u8>> {
        self.entries.get(hash).map(|e| e.data.clone())
    }

    fn touch(&mut self, hash: &BlockHash) -> bool {
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.touch();
            true
        } else {
            false
        }
    }

    fn put(&mut self, hash: BlockHash, data: Vec<u8>) -> usize {
        let data_size = data.len();

        // If entry already exists, update size accounting
        if let Some(old_entry) = self.entries.remove(&hash) {
            self.current_size -= old_entry.data.len();
        }

        // Evict entries if necessary
        let mut evicted = 0usize;
        while self.current_size + data_size > self.max_size && !self.entries.is_empty() {
            if self.evict_lru() {
                evicted += 1;
            }
        }

        // Only insert if it fits
        if data_size <= self.max_size {
            self.current_size += data_size;
            self.entries.insert(hash, CacheEntry::new(data));
        }
        evicted
    }

    fn evict_lru(&mut self) -> bool {
        // TODO: optimize to O(1) with a linked list or ordered structure for LRU tracking
        if let Some((oldest_hash, _)) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_access)
        {
            let oldest_hash = *oldest_hash;
            if let Some(entry) = self.entries.remove(&oldest_hash) {
                self.current_size -= entry.data.len();
                return true;
            }
        }
        false
    }

    fn contains(&self, hash: &BlockHash) -> bool {
        self.entries.contains_key(hash)
    }

    fn remove(&mut self, hash: &BlockHash) {
        if let Some(entry) = self.entries.remove(hash) {
            self.current_size -= entry.data.len();
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.current_size = 0;
    }
}

/// Block store with LRU caching
///
/// Provides content-addressed storage for blocks (file chunks).
/// Frequently accessed blocks are cached in memory for fast retrieval.
#[derive(Debug)]
pub struct CachedBlockStore {
    /// Underlying KV store for blocks
    store: SledStore,
    /// Metadata store for file info
    metadata: MetadataStore,
    /// LRU cache for blocks
    cache: Arc<RwLock<LruCache>>,
    /// Cache hit/miss statistics
    stats: Arc<RwLock<CacheStats>>,
}

/// Cache performance statistics
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    /// Number of cache hits
    pub hits: u64,
    /// Number of cache misses
    pub misses: u64,
    /// Number of blocks evicted from cache
    pub evictions: u64,
}

impl CacheStats {
    /// Calculate cache hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Key prefix for blocks in the store
const BLOCK_KEY_PREFIX: &[u8] = b"block:";

/// Create a storage key for a block hash
fn make_block_key(hash: &BlockHash) -> Vec<u8> {
    let mut key = BLOCK_KEY_PREFIX.to_vec();
    key.extend_from_slice(hash.as_bytes());
    key
}

impl CachedBlockStore {
    /// Create a new block store
    ///
    /// # Arguments
    /// * `store` - The underlying sled store
    /// * `cache_size` - Maximum size of the LRU cache in bytes
    ///
    /// # Returns
    /// A new `CachedBlockStore` instance
    pub fn new(store: SledStore, cache_size: usize) -> Self {
        let metadata_tree = store.open_tree("metadata")
            .unwrap_or_else(|_| panic!("Failed to open metadata tree"));
        let metadata = MetadataStore::from_tree(metadata_tree);

        Self {
            store,
            metadata,
            cache: Arc::new(RwLock::new(LruCache::new(cache_size))),
            stats: Arc::new(RwLock::new(CacheStats::default())),
        }
    }

    /// Create a new block store with both block and metadata storage
    ///
    /// # Arguments
    /// * `block_store` - Store for block data
    /// * `metadata_store` - Store for file metadata
    /// * `cache_size` - Maximum size of the LRU cache in bytes
    pub fn with_metadata(block_store: SledStore, metadata_store: SledStore, cache_size: usize) -> Self {
        Self {
            store: block_store,
            metadata: MetadataStore::new(metadata_store),
            cache: Arc::new(RwLock::new(LruCache::new(cache_size))),
            stats: Arc::new(RwLock::new(CacheStats::default())),
        }
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> CacheStats {
        self.stats.read().await.clone()
    }

    /// Clear the cache
    pub async fn clear_cache(&self) {
        self.cache.write().await.clear();
    }

    /// Get cache size in bytes
    pub async fn cache_size(&self) -> usize {
        self.cache.read().await.current_size
    }

    /// Prefetch blocks into cache
    pub async fn prefetch(&self, hashes: &[BlockHash]) -> Result<()> {
        for hash in hashes {
            let key = make_block_key(hash);
            if let Some(data) = self.store.get(&key).map_err(|e| {
                SyncthingError::Storage(format!("Failed to get block for prefetch: {}", e))
            })? {
                let mut cache = self.cache.write().await;
                if !cache.contains(hash) {
                    let evicted = cache.put(*hash, data);
                    let mut stats = self.stats.write().await;
                    stats.evictions += evicted as u64;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BlockStore for CachedBlockStore {
    async fn put(&self, hash: BlockHash, data: &[u8]) -> Result<()> {
        // Verify hash matches data
        let computed_hash = BlockHash::from_data(data);
        if computed_hash != hash {
            return Err(SyncthingError::protocol(format!(
                "Hash mismatch: expected {}, computed {}",
                hash, computed_hash
            )));
        }

        let key = make_block_key(&hash);

        // Store in sled
        self.store.put(&key, data).map_err(|e| {
            SyncthingError::Storage(format!("Failed to store block: {}", e))
        })?;

        // Add to cache
        let mut cache = self.cache.write().await;
        let evicted = cache.put(hash, data.to_vec());
        let mut stats = self.stats.write().await;
        stats.evictions += evicted as u64;

        Ok(())
    }

    async fn get(&self, hash: BlockHash) -> Result<Option<Vec<u8>>> {
        // Check cache first with read lock
        {
            let cache = self.cache.read().await;
            if let Some(data) = cache.peek(&hash) {
                drop(cache);
                let mut cache = self.cache.write().await;
                cache.touch(&hash);
                let mut stats = self.stats.write().await;
                stats.hits += 1;
                return Ok(Some(data));
            }
        }

        // Update miss stats
        {
            let mut stats = self.stats.write().await;
            stats.misses += 1;
        }

        // Fetch from store
        let key = make_block_key(&hash);
        match self.store.get(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to get block: {}", e))
        })? {
            Some(data) => {
                // Add to cache
                let mut cache = self.cache.write().await;
                let evicted = cache.put(hash, data.clone());
                let mut stats = self.stats.write().await;
                stats.evictions += evicted as u64;
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    async fn has(&self, hash: BlockHash) -> Result<bool> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if cache.contains(&hash) {
                return Ok(true);
            }
        }

        // Check store
        let key = make_block_key(&hash);
        self.store.contains(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to check block: {}", e))
        })
    }

    async fn delete(&self, hash: BlockHash) -> Result<()> {
        let key = make_block_key(&hash);

        // Remove from store
        self.store.delete(&key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to delete block: {}", e))
        })?;

        // Remove from cache
        let mut cache = self.cache.write().await;
        cache.remove(&hash);

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

/// Builder for creating a CachedBlockStore
pub struct BlockStoreBuilder {
    db_path: Option<std::path::PathBuf>,
    cache_size: usize,
}

impl Default for BlockStoreBuilder {
    fn default() -> Self {
        Self {
            db_path: None,
            cache_size: 64 * 1024 * 1024, // 64 MB default
        }
    }
}

impl BlockStoreBuilder {
    /// Create a new builder with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the database path
    pub fn path<P: AsRef<std::path::Path>>(mut self, path: P) -> Self {
        self.db_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set the cache size in bytes
    pub fn cache_size(mut self, size: usize) -> Self {
        self.cache_size = size;
        self
    }

    /// Build the block store
    pub fn build(self) -> Result<CachedBlockStore> {
        let store = match self.db_path {
            Some(path) => SledStore::open(path).map_err(|e| {
                SyncthingError::Storage(format!("Failed to open database: {}", e))
            })?,
            None => SledStore::open_in_memory().map_err(|e| {
                SyncthingError::Storage(format!("Failed to create in-memory database: {}", e))
            })?,
        };

        Ok(CachedBlockStore::new(store, self.cache_size))
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_block_store() -> (CachedBlockStore, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let store = SledStore::open(temp_dir.path()).unwrap();
        let block_store = CachedBlockStore::new(store, 1024 * 1024); // 1MB cache
        (block_store, temp_dir)
    }

    #[tokio::test]
    async fn test_block_store_put_get() {
        let (store, _temp) = create_test_block_store();

        let data = b"hello world";
        let hash = BlockHash::from_data(data);

        // Store block
        store.put(hash, data).await.unwrap();

        // Retrieve block
        let retrieved = store.get(hash).await.unwrap();
        assert_eq!(retrieved, Some(data.to_vec()));
    }

    #[tokio::test]
    async fn test_block_store_has() {
        let (store, _temp) = create_test_block_store();

        let data = b"test data";
        let hash = BlockHash::from_data(data);

        assert!(!store.has(hash).await.unwrap());

        store.put(hash, data).await.unwrap();

        assert!(store.has(hash).await.unwrap());
    }

    #[tokio::test]
    async fn test_block_store_delete() {
        let (store, _temp) = create_test_block_store();

        let data = b"delete me";
        let hash = BlockHash::from_data(data);

        store.put(hash, data).await.unwrap();
        assert!(store.has(hash).await.unwrap());

        store.delete(hash).await.unwrap();
        assert!(!store.has(hash).await.unwrap());
        assert_eq!(store.get(hash).await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_block_store_invalid_hash() {
        let (store, _temp) = create_test_block_store();

        let data = b"test data";
        let wrong_hash = BlockHash::from_bytes([0u8; 32]);

        let result = store.put(wrong_hash, data).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cache_functionality() {
        let (store, _temp) = create_test_block_store();

        let data = b"cached data";
        let hash = BlockHash::from_data(data);

        // Put data (also adds to cache)
        store.put(hash, data).await.unwrap();

        // First get should be cache hit (because put adds to cache)
        let _ = store.get(hash).await.unwrap();
        let stats = store.cache_stats().await;
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 0);

        // Get again (should also be cache hit)
        let _ = store.get(hash).await.unwrap();
        let stats = store.cache_stats().await;
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.hit_rate(), 1.0);
    }

    #[tokio::test]
    async fn test_cache_eviction() {
        let temp_dir = TempDir::new().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let store = CachedBlockStore::new(sled_store, 100); // Very small cache

        // Store multiple blocks that exceed cache size
        for i in 0..10 {
            let data = vec![i as u8; 20]; // 20 bytes each
            let hash = BlockHash::from_data(&data);
            store.put(hash, &data).await.unwrap();
        }

        // Cache should have evicted some entries
        let cache_size = store.cache_size().await;
        assert!(cache_size <= 100);
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let (store, _temp) = create_test_block_store();

        let data = b"clear me";
        let hash = BlockHash::from_data(data);

        store.put(hash, data).await.unwrap();
        store.get(hash).await.unwrap(); // Populate cache

        store.clear_cache().await;

        // After clear, should be a cache miss
        let _ = store.get(hash).await.unwrap();
        let stats = store.cache_stats().await;
        // One miss from initial get, one miss after clear
        assert!(stats.misses >= 1);
    }

    #[tokio::test]
    async fn test_prefetch() {
        let (store, _temp) = create_test_block_store();

        let mut hashes = Vec::new();
        for i in 0..5 {
            let data = vec![i as u8; 100];
            let hash = BlockHash::from_data(&data);
            hashes.push(hash);
            store.put(hash, &data).await.unwrap();
        }

        store.clear_cache().await;

        // Prefetch should load into cache
        store.prefetch(&hashes).await.unwrap();

        // All should be cache hits now
        for hash in hashes {
            let _ = store.get(hash).await.unwrap();
        }

        let stats = store.cache_stats().await;
        // Prefetch puts items in cache without counting as hits
        // Subsequent gets should be hits
        assert_eq!(stats.hits, 5);
    }

    #[tokio::test]
    async fn test_folder_index_operations() {
        let (store, _temp) = create_test_block_store();

        let folder = FolderId::new("test-folder");

        // Initially empty
        let index = store.get_index(&folder).await.unwrap();
        assert!(index.is_empty());

        // Update index
        let files = vec![
            FileInfo::new("file1.txt"),
            FileInfo::new("file2.txt"),
        ];

        store.update_index(&folder, files.clone()).await.unwrap();

        let index = store.get_index(&folder).await.unwrap();
        assert_eq!(index.len(), 2);

        // Delta update
        let mut new_file = FileInfo::new("file3.txt");
        new_file.sequence = 1;
        store.update_index_delta(&folder, vec![new_file]).await.unwrap();

        let index = store.get_index(&folder).await.unwrap();
        assert_eq!(index.len(), 3);
    }

    #[tokio::test]
    async fn test_folder_stats() {
        let (store, _temp) = create_test_block_store();

        let folder = FolderId::new("test-folder");

        let stats = store.folder_stats(&folder).await.unwrap();
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.total_bytes, 0);

        // Add files and update index
        let mut file = FileInfo::new("test.txt");
        file.size = 1000;
        store.update_index(&folder, vec![file]).await.unwrap();

        let stats = store.folder_stats(&folder).await.unwrap();
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.total_bytes, 1000);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let (store, _temp) = create_test_block_store();
        let store = Arc::new(store);

        let mut handles = vec![];

        // Spawn multiple tasks that write blocks
        for i in 0..10 {
            let store = store.clone();
            let handle = tokio::spawn(async move {
                let data = vec![i as u8; 1000];
                let hash = BlockHash::from_data(&data);
                store.put(hash, &data).await.unwrap();
                hash
            });
            handles.push(handle);
        }

        let hashes: Vec<BlockHash> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Verify all blocks are accessible
        for hash in hashes {
            assert!(store.has(hash).await.unwrap());
        }
    }
}
