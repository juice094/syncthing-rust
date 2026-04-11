//! Module: syncthing-db
//! Worker: Agent-E
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Key-value storage abstraction
//!
//! This module provides a synchronous key-value store interface
//! backed by sled for embedded storage.

use std::path::Path;

use syncthing_core::SyncthingError;

/// Result type for KV store operations
pub type Result<T> = std::result::Result<T, SyncthingError>;

/// Sled-based KV store
///
/// Provides a simple interface for storing and retrieving
/// byte arrays keyed by byte arrays.
#[derive(Debug)]
pub struct SledStore {
    db: sled::Db,
}

impl SledStore {
    /// Open database at path
    ///
    /// Creates the database directory if it doesn't exist.
    ///
    /// # Arguments
    /// * `path` - Path to the database directory
    ///
    /// # Returns
    /// A new `SledStore` instance or an error
    ///
    /// # Errors
    /// Returns an error if the database cannot be opened
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path).map_err(|e| {
            SyncthingError::Storage(format!("Failed to open database: {}", e))
        })?;
        Ok(Self { db })
    }

    /// Create an in-memory database
    ///
    /// Useful for testing and temporary storage.
    pub fn open_in_memory() -> Result<Self> {
        let db = sled::Config::new()
            .temporary(true)
            .open()
            .map_err(|e| {
                SyncthingError::Storage(format!("Failed to create in-memory database: {}", e))
            })?;
        Ok(Self { db })
    }

    /// Get value by key
    ///
    /// # Arguments
    /// * `key` - The key to look up
    ///
    /// # Returns
    /// `Ok(Some(value))` if the key exists, `Ok(None)` if not
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let result = self.db.get(key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to get key: {}", e))
        })?;
        Ok(result.map(|v| v.to_vec()))
    }

    /// Store value by key
    ///
    /// # Arguments
    /// * `key` - The key to store
    /// * `value` - The value to store
    ///
    /// # Errors
    /// Returns an error if the write fails
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.db.insert(key, value).map_err(|e| {
            SyncthingError::Storage(format!("Failed to put key: {}", e))
        })?;
        Ok(())
    }

    /// Delete a key-value pair
    ///
    /// # Arguments
    /// * `key` - The key to delete
    ///
    /// # Returns
    /// `Ok(true)` if the key was present and deleted, `Ok(false)` if not found
    pub fn delete(&self, key: &[u8]) -> Result<bool> {
        let existed = self.db.remove(key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to delete key: {}", e))
        })?;
        Ok(existed.is_some())
    }

    /// Check if a key exists
    ///
    /// # Arguments
    /// * `key` - The key to check
    pub fn contains(&self, key: &[u8]) -> Result<bool> {
        self.db.contains_key(key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to check key: {}", e))
        })
    }

    /// Apply a batch of operations atomically
    ///
    /// All operations in the batch are applied together or not at all.
    ///
    /// # Arguments
    /// * `batch` - Vector of (key, Some(value)) for inserts or (key, None) for deletes
    pub fn apply_batch(&self, batch: Vec<(Vec<u8>, Option<Vec<u8>>)>) -> Result<()> {
        let mut sled_batch = sled::Batch::default();

        for (key, value) in batch {
            match value {
                Some(v) => sled_batch.insert(key, v),
                None => sled_batch.remove(key),
            }
        }

        self.db.apply_batch(sled_batch).map_err(|e| {
            SyncthingError::Storage(format!("Failed to apply batch: {}", e))
        })?;

        Ok(())
    }

    /// Get all keys with a given prefix
    ///
    /// # Arguments
    /// * `prefix` - The prefix to scan for
    pub fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut results = Vec::new();
        for result in self.db.scan_prefix(prefix) {
            let (key, value) = result.map_err(|e| {
                SyncthingError::Storage(format!("Failed to scan: {}", e))
            })?;
            results.push((key.to_vec(), value.to_vec()));
        }
        Ok(results)
    }

    /// Create a tree (namespace) within the database
    ///
    /// Trees provide separate key namespaces within the same database.
    pub fn open_tree(&self, name: &str) -> Result<SledTree> {
        let tree = self.db.open_tree(name).map_err(|e| {
            SyncthingError::Storage(format!("Failed to open tree '{}': {}", name, e))
        })?;
        Ok(SledTree { tree })
    }

    /// Flush all pending writes to disk
    pub fn flush(&self) -> Result<()> {
        self.db.flush().map_err(|e| {
            SyncthingError::Storage(format!("Failed to flush database: {}", e))
        })?;
        Ok(())
    }
}

/// A tree (namespace) within the database
#[derive(Debug)]
pub struct SledTree {
    tree: sled::Tree,
}

impl SledTree {
    /// Get value by key
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let result = self.tree.get(key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to get key from tree: {}", e))
        })?;
        Ok(result.map(|v| v.to_vec()))
    }

    /// Store value by key
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.tree.insert(key, value).map_err(|e| {
            SyncthingError::Storage(format!("Failed to put key in tree: {}", e))
        })?;
        Ok(())
    }

    /// Delete a key-value pair
    pub fn delete(&self, key: &[u8]) -> Result<bool> {
        let existed = self.tree.remove(key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to delete key from tree: {}", e))
        })?;
        Ok(existed.is_some())
    }

    /// Check if a key exists
    pub fn contains(&self, key: &[u8]) -> Result<bool> {
        self.tree.contains_key(key).map_err(|e| {
            SyncthingError::Storage(format!("Failed to check key in tree: {}", e))
        })
    }

    /// Get all keys with a given prefix
    pub fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut results = Vec::new();
        for result in self.tree.scan_prefix(prefix) {
            let (key, value) = result.map_err(|e| {
                SyncthingError::Storage(format!("Failed to scan tree: {}", e))
            })?;
            results.push((key.to_vec(), value.to_vec()));
        }
        Ok(results)
    }

    /// Apply a batch of operations atomically
    pub fn apply_batch(&self, batch: Vec<(Vec<u8>, Option<Vec<u8>>)>) -> Result<()> {
        let mut sled_batch = sled::Batch::default();

        for (key, value) in batch {
            match value {
                Some(v) => sled_batch.insert(key, v),
                None => sled_batch.remove(key),
            }
        }

        self.tree.apply_batch(sled_batch).map_err(|e| {
            SyncthingError::Storage(format!("Failed to apply batch to tree: {}", e))
        })?;

        Ok(())
    }

    /// Flush pending writes to disk
    pub fn flush(&self) -> Result<()> {
        self.tree.flush().map_err(|e| {
            SyncthingError::Storage(format!("Failed to flush tree: {}", e))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_sled_store_basic() {
        let temp_dir = TempDir::new().unwrap();
        let store = SledStore::open(temp_dir.path()).unwrap();

        // Test put and get
        store.put(b"key1", b"value1").unwrap();
        let value = store.get(b"key1").unwrap();
        assert_eq!(value, Some(b"value1".to_vec()));

        // Test non-existent key
        let value = store.get(b"nonexistent").unwrap();
        assert_eq!(value, None);

        // Test contains
        assert!(store.contains(b"key1").unwrap());
        assert!(!store.contains(b"nonexistent").unwrap());

        // Test delete
        assert!(store.delete(b"key1").unwrap());
        assert!(!store.contains(b"key1").unwrap());
        assert!(!store.delete(b"key1").unwrap());
    }

    #[test]
    fn test_sled_store_batch() {
        let temp_dir = TempDir::new().unwrap();
        let store = SledStore::open(temp_dir.path()).unwrap();

        let batch = vec![
            (b"key1".to_vec(), Some(b"value1".to_vec())),
            (b"key2".to_vec(), Some(b"value2".to_vec())),
            (b"key3".to_vec(), Some(b"value3".to_vec())),
        ];

        store.apply_batch(batch).unwrap();

        assert_eq!(store.get(b"key1").unwrap(), Some(b"value1".to_vec()));
        assert_eq!(store.get(b"key2").unwrap(), Some(b"value2".to_vec()));
        assert_eq!(store.get(b"key3").unwrap(), Some(b"value3".to_vec()));

        // Test batch delete
        let batch = vec![
            (b"key1".to_vec(), None),
            (b"key2".to_vec(), None),
        ];

        store.apply_batch(batch).unwrap();

        assert_eq!(store.get(b"key1").unwrap(), None);
        assert_eq!(store.get(b"key2").unwrap(), None);
        assert_eq!(store.get(b"key3").unwrap(), Some(b"value3".to_vec()));
    }

    #[test]
    fn test_sled_store_scan_prefix() {
        let temp_dir = TempDir::new().unwrap();
        let store = SledStore::open(temp_dir.path()).unwrap();

        store.put(b"prefix:key1", b"value1").unwrap();
        store.put(b"prefix:key2", b"value2").unwrap();
        store.put(b"other:key3", b"value3").unwrap();

        let results = store.scan_prefix(b"prefix:").unwrap();
        assert_eq!(results.len(), 2);

        let keys: Vec<_> = results.iter().map(|(k, _)| String::from_utf8_lossy(k).to_string()).collect();
        assert!(keys.contains(&"prefix:key1".to_string()));
        assert!(keys.contains(&"prefix:key2".to_string()));
    }

    #[test]
    fn test_sled_tree() {
        let temp_dir = TempDir::new().unwrap();
        let store = SledStore::open(temp_dir.path()).unwrap();
        let tree = store.open_tree("test_tree").unwrap();

        tree.put(b"tree_key", b"tree_value").unwrap();
        assert_eq!(tree.get(b"tree_key").unwrap(), Some(b"tree_value".to_vec()));
        assert!(tree.contains(b"tree_key").unwrap());

        tree.delete(b"tree_key").unwrap();
        assert!(!tree.contains(b"tree_key").unwrap());
    }

    #[test]
    fn test_sled_tree_isolation() {
        let temp_dir = TempDir::new().unwrap();
        let store = SledStore::open(temp_dir.path()).unwrap();

        let tree1 = store.open_tree("tree1").unwrap();
        let tree2 = store.open_tree("tree2").unwrap();

        tree1.put(b"key", b"value1").unwrap();
        tree2.put(b"key", b"value2").unwrap();

        assert_eq!(tree1.get(b"key").unwrap(), Some(b"value1".to_vec()));
        assert_eq!(tree2.get(b"key").unwrap(), Some(b"value2".to_vec()));
        assert_eq!(store.get(b"key").unwrap(), None);
    }

    #[tokio::test]
    async fn test_sled_store_concurrent() {
        let temp_dir = TempDir::new().unwrap();
        let store = std::sync::Arc::new(SledStore::open(temp_dir.path()).unwrap());

        let mut handles = vec![];

        for i in 0..10 {
            let store = store.clone();
            let handle = tokio::spawn(async move {
                let key = format!("key{}", i);
                let value = format!("value{}", i);
                store.put(key.as_bytes(), value.as_bytes()).unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        for i in 0..10 {
            let key = format!("key{}", i);
            let value = format!("value{}", i);
            assert_eq!(store.get(key.as_bytes()).unwrap(), Some(value.into_bytes()));
        }
    }
}
