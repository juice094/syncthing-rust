# Agent-E Task: Storage Layer

## ⚠️ CRITICAL RULES

1. **DO NOT MODIFY** `syncthing-core` crate - it is READ ONLY
2. **ALL DELIVERIES** must be marked `UNVERIFIED`
3. **MUST IMPLEMENT** `BlockStore` trait
4. **Data integrity** is critical

## Task Overview

Implement storage layer in `crates/syncthing-db/`.

## Deliverables

### 1. kv.rs - Key-Value Store

```rust
//! Key-value storage abstraction
//! 
//! ⚠️ STATUS: UNVERIFIED

use sled::Db;

/// Sled-based KV store
pub struct SledStore {
    db: Db,
}

impl SledStore {
    /// Open database at path
    pub fn open(path: &Path) -> Result<Self> {
        // UNVERIFIED
    }
    
    /// Get value
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // UNVERIFIED
    }
    
    /// Put value
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        // UNVERIFIED
    }
}
```

### 2. metadata.rs - Metadata Storage

```rust
//! Metadata storage for files and folders
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{FileInfo, FolderId, Result};

/// Metadata store
pub struct MetadataStore {
    // fields
}

impl MetadataStore {
    /// Store file info
    pub async fn put_file(&self, folder: &FolderId, info: &FileInfo) -> Result<()> {
        // UNVERIFIED
    }
    
    /// Get file info
    pub async fn get_file(&self, folder: &FolderId, name: &str) -> Result<Option<FileInfo>> {
        // UNVERIFIED
    }
    
    /// Get all files in folder
    pub async fn get_folder_index(&self, folder: &FolderId) -> Result<Vec<FileInfo>> {
        // UNVERIFIED
    }
}
```

### 3. block_cache.rs - Block Cache

```rust
//! Block storage and caching
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{traits::BlockStore, BlockHash, Result};
use async_trait::async_trait;

/// Block store with LRU cache
pub struct CachedBlockStore {
    // fields
}

#[async_trait]
impl BlockStore for CachedBlockStore {
    // Implement ALL trait methods
}
```

## Requirements

1. Use `sled` for embedded storage
2. Content-addressed block storage
3. LRU cache for frequently accessed blocks
4. Atomic batch operations

## Testing

- Test data persistence
- Test cache eviction
- Test concurrent access
