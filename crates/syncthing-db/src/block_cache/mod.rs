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

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use syncthing_core::{
    traits::{BlockStore, FolderStats},
    BlockHash, FileInfo, FolderId, Result, SyncthingError,
};

use crate::blocking::run_blocking;
use crate::kv::SledStore;
use crate::metadata::MetadataStore;

mod lru;
use lru::LruCache;

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
    pub fn new(store: SledStore, cache_size: usize) -> Result<Self> {
        let metadata_tree = store
            .open_tree("metadata")
            .map_err(|e| SyncthingError::Storage(format!("Failed to open metadata tree: {}", e)))?;
        let metadata = MetadataStore::from_tree(metadata_tree);

        Ok(Self {
            store,
            metadata,
            cache: Arc::new(RwLock::new(LruCache::new(cache_size))),
            stats: Arc::new(RwLock::new(CacheStats::default())),
        })
    }

    /// Create a new block store with both block and metadata storage
    ///
    /// # Arguments
    /// * `block_store` - Store for block data
    /// * `metadata_store` - Store for file metadata
    /// * `cache_size` - Maximum size of the LRU cache in bytes
    pub fn with_metadata(
        block_store: SledStore,
        metadata_store: SledStore,
        cache_size: usize,
    ) -> Self {
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
            let store = self.store.clone();
            if let Some(data) = run_blocking("block store prefetch", move || {
                store.get(&key).map_err(|e| {
                    SyncthingError::Storage(format!("Failed to get block for prefetch: {}", e))
                })
            })
            .await?
            {
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

        // Store in sled on a blocking thread to avoid stalling the tokio runtime
        let store = self.store.clone();
        let data_vec = data.to_vec();
        run_blocking("block store put", move || {
            store
                .put(&key, &data_vec)
                .map_err(|e| SyncthingError::Storage(format!("Failed to store block: {}", e)))
        })
        .await?;

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

        // Fetch from store on a blocking thread
        let key = make_block_key(&hash);
        let store = self.store.clone();
        match run_blocking("block store get", move || {
            store
                .get(&key)
                .map_err(|e| SyncthingError::Storage(format!("Failed to get block: {}", e)))
        })
        .await?
        {
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

        // Check store on a blocking thread
        let key = make_block_key(&hash);
        let store = self.store.clone();
        run_blocking("block store contains", move || {
            store
                .contains(&key)
                .map_err(|e| SyncthingError::Storage(format!("Failed to check block: {}", e)))
        })
        .await
    }

    async fn delete(&self, hash: BlockHash) -> Result<()> {
        let key = make_block_key(&hash);

        // Remove from store on a blocking thread
        let store = self.store.clone();
        run_blocking("block store delete", move || {
            store
                .delete(&key)
                .map_err(|e| SyncthingError::Storage(format!("Failed to delete block: {}", e)))
        })
        .await?;

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
            Some(path) => SledStore::open(path)
                .map_err(|e| SyncthingError::Storage(format!("Failed to open database: {}", e)))?,
            None => SledStore::open_in_memory().map_err(|e| {
                SyncthingError::Storage(format!("Failed to create in-memory database: {}", e))
            })?,
        };

        CachedBlockStore::new(store, self.cache_size)
    }
}

#[cfg(test)]
mod tests;
