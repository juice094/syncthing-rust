//! Module: syncthing-db
//! Worker: Agent-E
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Database and Storage Layer for Syncthing
//!
//! This crate provides the storage layer for Syncthing, including:
//!
//! - **KV Store** (`kv.rs`): Low-level key-value storage using sled
//! - **Metadata Store** (`metadata.rs`): File and folder metadata storage
//! - **Block Cache** (`block_cache.rs`): Content-addressed block storage with LRU caching
//! - **Store** (`store.rs`): BlockStore trait implementation
//!
//! # Usage
//!
//! ```rust,no_run
//! use syncthing_db::{CachedBlockStore, SledStore};
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Open the database
//! let store = SledStore::open("./syncthing.db")?;
//!
//! // Create a block store with 64MB cache
//! let block_store = CachedBlockStore::new(store, 64 * 1024 * 1024);
//!
//! // Use the block store (implements BlockStore trait)
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

pub mod block_cache;
pub mod kv;
pub mod metadata;
pub mod store;

// Re-export main types
pub use block_cache::{CachedBlockStore, CacheStats, BlockStoreBuilder};
pub use kv::{SledStore, SledTree};
pub use metadata::{MetadataStore, FolderStats as DbFolderStats};
pub use store::BlockStoreImpl;

/// Default database path
pub const DEFAULT_DB_PATH: &str = "./syncthing.db";

/// Default cache size (64 MB)
pub const DEFAULT_CACHE_SIZE: usize = 64 * 1024 * 1024;

/// Version of this crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Create a new block store with default settings
///
/// # Arguments
/// * `path` - Path to the database directory
///
/// # Returns
/// A configured `CachedBlockStore` instance
///
/// # Errors
/// Returns an error if the database cannot be opened
pub fn create_block_store<P: AsRef<std::path::Path>>(path: P) -> crate::kv::Result<CachedBlockStore> {
    let store = SledStore::open(path)?;
    Ok(CachedBlockStore::new(store, DEFAULT_CACHE_SIZE))
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use syncthing_core::{BlockHash, FileInfo, FolderId};
    use syncthing_core::traits::BlockStore;

    #[tokio::test]
    async fn test_full_workflow() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let block_store = CachedBlockStore::new(sled_store, 1024 * 1024);

        let folder = FolderId::new("test-folder");

        // 1. Store some blocks
        let block1_data = b"block 1 content";
        let block1_hash = BlockHash::from_data(block1_data);
        block_store.put(block1_hash, block1_data).await.unwrap();

        let block2_data = b"block 2 content";
        let block2_hash = BlockHash::from_data(block2_data);
        block_store.put(block2_hash, block2_data).await.unwrap();

        // 2. Create and store file metadata
        let mut file = FileInfo::new("test.txt");
        file.size = (block1_data.len() + block2_data.len()) as u64;
        file.blocks = vec![
            syncthing_core::BlockInfo {
                hash: block1_hash,
                offset: 0,
                size: block1_data.len(),
            },
            syncthing_core::BlockInfo {
                hash: block2_hash,
                offset: block1_data.len() as u64,
                size: block2_data.len(),
            },
        ];

        block_store.update_index(&folder, vec![file.clone()]).await.unwrap();

        // 3. Verify index
        let index = block_store.get_index(&folder).await.unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].name, "test.txt");

        // 4. Verify blocks can be retrieved
        let retrieved1 = block_store.get(block1_hash).await.unwrap();
        assert_eq!(retrieved1, Some(block1_data.to_vec()));

        let retrieved2 = block_store.get(block2_hash).await.unwrap();
        assert_eq!(retrieved2, Some(block2_data.to_vec()));

        // 5. Check folder stats
        let stats = block_store.folder_stats(&folder).await.unwrap();
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.total_bytes, file.size);
    }

    #[tokio::test]
    async fn test_block_integrity() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let block_store = CachedBlockStore::new(sled_store, 1024 * 1024);

        let data = b"important data that must be stored correctly";
        let hash = BlockHash::from_data(data);

        // Store the block
        block_store.put(hash, data).await.unwrap();

        // Simulate cache clear (to force reading from disk)
        block_store.clear_cache().await;

        // Verify data integrity
        let retrieved = block_store.get(hash).await.unwrap();
        assert_eq!(retrieved, Some(data.to_vec()));
    }

    #[tokio::test]
    async fn test_delta_updates() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sled_store = SledStore::open(temp_dir.path()).unwrap();
        let block_store = CachedBlockStore::new(sled_store, 1024 * 1024);

        let folder = FolderId::new("test-folder");

        // Initial index
        let file1 = FileInfo::new("file1.txt");
        let file2 = FileInfo::new("file2.txt");
        block_store.update_index(&folder, vec![file1, file2]).await.unwrap();

        // Delta update - add file3, modify file1
        let mut file1_updated = FileInfo::new("file1.txt");
        file1_updated.size = 1000;
        let file3 = FileInfo::new("file3.txt");

        block_store.update_index_delta(&folder, vec![file1_updated, file3]).await.unwrap();

        // Verify all three files exist
        let index = block_store.get_index(&folder).await.unwrap();
        assert_eq!(index.len(), 3);

        // Verify file1 was updated
        let file1_retrieved = index.iter().find(|f| f.name == "file1.txt").unwrap();
        assert_eq!(file1_retrieved.size, 1000);
    }
}
