# Agent-B Task: Filesystem Abstraction

## ⚠️ CRITICAL RULES

1. **DO NOT MODIFY** `syncthing-core` crate - it is READ ONLY
2. **ALL DELIVERIES** must be marked `UNVERIFIED`
3. **MUST IMPLEMENT** `FileSystem` trait from `syncthing-core::traits`
4. **Cross-platform support** required (Windows, macOS, Linux)

## Task Overview

Implement filesystem abstraction in `crates/syncthing-fs/`.

## Deliverables

### 1. filesystem.rs - FileSystem Trait Implementation

```rust
//! Filesystem implementation
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{
    traits::FileSystem,
    BlockHash, FileInfo, Result,
};
use async_trait::async_trait;
use std::path::Path;

/// Native filesystem implementation
pub struct NativeFileSystem {
    root: std::path::PathBuf,
}

impl NativeFileSystem {
    /// Create new filesystem at given root
    pub fn new(root: impl AsRef<Path>) -> Self {
        // UNVERIFIED
    }
}

#[async_trait]
impl FileSystem for NativeFileSystem {
    // Implement ALL trait methods
}
```

### 2. scanner.rs - File Scanner

```rust
//! File scanner with block hashing
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{BlockHash, BlockInfo, FileInfo, Result};

/// Scan a file and compute block hashes
pub async fn scan_file(
    path: &Path,
    block_size: usize,
) -> Result<FileInfo> {
    // UNVERIFIED implementation
}

/// Compute SHA-256 hash for data block
pub fn hash_block(data: &[u8]) -> BlockHash {
    BlockHash::from_data(data)
}
```

### 3. watcher.rs - Filesystem Watcher

```rust
//! Filesystem change watcher
//! 
//! ⚠️ STATUS: UNVERIFIED

use notify::{Watcher, RecursiveMode};

/// Watch a folder for changes
pub struct FolderWatcher {
    // fields
}

impl FolderWatcher {
    /// Create new watcher
    pub fn new(
        path: &Path,
        callback: impl Fn(FsEvent) + Send + 'static,
    ) -> Result<Self> {
        // UNVERIFIED
    }
}

#[derive(Debug)]
pub enum FsEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}
```

### 4. ignore.rs - .stignore Parser

```rust
//! .stignore file parser
//! 
//! ⚠️ STATUS: UNVERIFIED

/// Pattern matcher for ignore rules
pub struct IgnorePatterns {
    patterns: Vec<Pattern>,
}

impl IgnorePatterns {
    /// Load from .stignore file
    pub fn from_file(path: &Path) -> Result<Self> {
        // UNVERIFIED
    }
    
    /// Check if path matches any ignore pattern
    pub fn is_ignored(&self, path: &Path) -> bool {
        // UNVERIFIED
    }
}
```

## Requirements

1. Handle Windows path separators correctly
2. Support symlinks (don't follow by default)
3. Atomic file writes (write to temp, then rename)
4. Preserve file permissions where possible

## Testing

- Unit tests for each module
- Use `tempfile` crate for test isolation
- Test cross-platform path handling
