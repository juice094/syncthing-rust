//! Local database interface.
//!
//! Provides local storage and querying of file metadata.
//!
//! See "docs/design/ADR-20261105-local-database-cache.md".

use crate::error::{Result, SyncError};
use dashmap::DashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use syncthing_core::types::{FileInfo, Folder, IndexID};
use tokio::sync::Mutex;

/// Abstraction over per-folder file metadata storage.
#[async_trait::async_trait]
pub trait LocalDatabase: Send + Sync {
    /// Returns a single file's metadata, if known.
    async fn get_file(&self, folder: &str, name: &str) -> Result<Option<FileInfo>>;

    /// Returns all file metadata known for a folder.
    async fn get_folder_files(&self, folder: &str) -> Result<Vec<FileInfo>>;

    /// Inserts or updates one file's metadata.
    async fn update_file(&self, folder: &str, info: FileInfo) -> Result<()>;

    /// Inserts or updates a batch of file metadata.
    async fn update_files(&self, folder: &str, files: Vec<FileInfo>) -> Result<()>;

    /// Removes a file's metadata.
    async fn delete_file(&self, folder: &str, name: &str) -> Result<()>;

    /// Returns `true` when the file is known locally.
    async fn has_file(&self, folder: &str, name: &str) -> Result<bool>;

    /// Returns a folder's configuration, if stored.
    async fn get_folder(&self, folder_id: &str) -> Result<Option<Folder>>;

    /// Stores a folder's configuration.
    async fn update_folder(&self, folder: Folder) -> Result<()>;

    /// Returns files whose sequence number is greater than `since`.
    async fn get_needed_files(&self, folder: &str, since: u64) -> Result<Vec<FileInfo>>;

    /// Returns all known versions of a file (global state check).
    async fn check_globals(&self, folder: &str, name: &str) -> Result<Vec<FileInfo>>;

    /// Returns the current per-folder sequence number.
    async fn get_sequence(&self, folder: &str) -> Result<u64>;

    /// Increments and returns the per-folder sequence number.
    async fn increment_sequence(&self, folder: &str) -> Result<u64>;

    /// Resets the per-folder sequence number to 0 (used on major changes /
    /// index rebuilds).
    async fn reset_sequence(&self, folder: &str) -> Result<()>;

    /// Returns the folder's index metadata `(index_id, max_sequence)`, if stored.
    async fn get_folder_index_meta(&self, folder: &str) -> Result<Option<(IndexID, u64)>>;

    /// Stores the folder's index metadata.
    async fn update_folder_index_meta(
        &self,
        folder: &str,
        index_id: IndexID,
        max_sequence: u64,
    ) -> Result<()>;
}

/// In-memory database implementation (intended for tests).
#[derive(Debug, Default)]
pub struct MemoryDatabase {
    files: DashMap<String, Vec<FileInfo>>,
    folders: DashMap<String, Folder>,
    sequences: DashMap<String, u64>,
    index_metas: DashMap<String, (IndexID, u64)>,
}

impl MemoryDatabase {
    /// Creates a new shared in-memory database.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait::async_trait]
impl LocalDatabase for MemoryDatabase {
    async fn get_file(&self, folder: &str, name: &str) -> Result<Option<FileInfo>> {
        Ok(self.files.get(folder).and_then(
            |files: dashmap::mapref::one::Ref<'_, String, Vec<FileInfo>>| {
                files.iter().find(|f| f.name == name).cloned()
            },
        ))
    }

    async fn get_folder_files(&self, folder: &str) -> Result<Vec<FileInfo>> {
        Ok(self
            .files
            .get(folder)
            .map(|f: dashmap::mapref::one::Ref<'_, String, Vec<FileInfo>>| f.clone())
            .unwrap_or_default())
    }

    async fn update_file(&self, folder: &str, info: FileInfo) -> Result<()> {
        let mut files = self.files.entry(folder.to_string()).or_default();
        if let Some(idx) = files.iter().position(|f| f.name == info.name) {
            files[idx] = info;
        } else {
            files.push(info);
        }
        Ok(())
    }

    async fn update_files(&self, folder: &str, new_files: Vec<FileInfo>) -> Result<()> {
        for file in new_files {
            self.update_file(folder, file).await?;
        }
        Ok(())
    }

    async fn delete_file(&self, folder: &str, name: &str) -> Result<()> {
        if let Some(mut files) = self.files.get_mut(folder) {
            files.retain(|f| f.name != name);
        }
        Ok(())
    }

    async fn has_file(&self, folder: &str, name: &str) -> Result<bool> {
        let result: Option<FileInfo> = self.get_file(folder, name).await?;
        Ok(result.is_some())
    }

    async fn get_folder(&self, folder_id: &str) -> Result<Option<Folder>> {
        Ok(self
            .folders
            .get(folder_id)
            .map(|f: dashmap::mapref::one::Ref<'_, String, Folder>| f.clone()))
    }

    async fn update_folder(&self, folder: Folder) -> Result<()> {
        self.folders.insert(folder.id.clone(), folder);
        Ok(())
    }

    async fn get_needed_files(&self, folder: &str, since: u64) -> Result<Vec<FileInfo>> {
        let files: Vec<FileInfo> = self.get_folder_files(folder).await?;
        Ok(files.into_iter().filter(|f| f.sequence > since).collect())
    }

    async fn check_globals(&self, folder: &str, name: &str) -> Result<Vec<FileInfo>> {
        // Simplified global check: returns all locally known versions of the
        // file. A full implementation would query the global database.
        if let Some(file) = self.get_file(folder, name).await? {
            Ok(vec![file])
        } else {
            Ok(vec![])
        }
    }

    async fn get_sequence(&self, folder: &str) -> Result<u64> {
        Ok(self.sequences.get(folder).map(|s| *s).unwrap_or(0))
    }

    async fn increment_sequence(&self, folder: &str) -> Result<u64> {
        let mut seq = self.sequences.entry(folder.to_string()).or_insert(0);
        *seq += 1;
        Ok(*seq)
    }

    async fn reset_sequence(&self, folder: &str) -> Result<()> {
        self.sequences.insert(folder.to_string(), 0);
        Ok(())
    }

    async fn get_folder_index_meta(&self, folder: &str) -> Result<Option<(IndexID, u64)>> {
        Ok(self.index_metas.get(folder).map(|m| *m))
    }

    async fn update_folder_index_meta(
        &self,
        folder: &str,
        index_id: IndexID,
        max_sequence: u64,
    ) -> Result<()> {
        self.index_metas
            .insert(folder.to_string(), (index_id, max_sequence));
        Ok(())
    }
}

/// Filesystem-backed database (the real persistent implementation).
///
/// See "docs/design/ADR-20261105-local-database-cache.md" for the in-memory
/// cache soft cap, batch flush, and sequence-locking design.
#[derive(Clone)]
pub struct FileSystemDatabase {
    base_path: std::path::PathBuf,
    cache: DashMap<String, Vec<FileInfo>>,
    cache_size: Arc<AtomicUsize>,
    cache_cap: usize,
    /// Per-folder mutex to serialize sequence read-modify-write.
    /// Prevents the "empty string" parse error caused by concurrent
    /// truncate-then-write in tokio::fs::write.
    seq_locks: DashMap<String, Arc<Mutex<()>>>,
}

impl FileSystemDatabase {
    /// Creates a new filesystem database rooted at `base_path`.
    pub fn new(base_path: impl AsRef<Path>) -> Arc<Self> {
        Arc::new(Self {
            base_path: base_path.as_ref().to_path_buf(),
            cache: DashMap::new(),
            cache_size: Arc::new(AtomicUsize::new(0)),
            cache_cap: 100_000,
            seq_locks: DashMap::new(),
        })
    }

    fn get_seq_lock(&self, folder: &str) -> Arc<Mutex<()>> {
        self.seq_locks
            .entry(folder.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Randomly evicts one folder's cache entry and returns the evicted file count.
    fn evict_one_folder(&self) -> usize {
        if let Some(entry) = self.cache.iter().next() {
            let key = entry.key().clone();
            let count = entry.value().len();
            drop(entry);
            self.cache.remove(&key);
            self.cache_size.fetch_sub(count, Ordering::Relaxed);
            count
        } else {
            0
        }
    }

    fn folder_path(&self, folder: &str) -> std::path::PathBuf {
        self.base_path.join(folder)
    }

    fn file_path(&self, folder: &str, name: &str) -> std::path::PathBuf {
        self.folder_path(folder)
            .join(format!("{}.json", name.replace('/', "_")))
    }
}

#[async_trait::async_trait]
impl LocalDatabase for FileSystemDatabase {
    async fn get_file(&self, folder: &str, name: &str) -> Result<Option<FileInfo>> {
        if let Some(files) = self.cache.get(folder) {
            if let Some(file) = files.iter().find(|f| f.name == name) {
                return Ok(Some(file.clone()));
            }
        }

        let path = self.file_path(folder, name);
        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let file: FileInfo = serde_json::from_str(&content)
            .map_err(|e| SyncError::database(format!("Failed to parse file info: {}", e)))?;

        Ok(Some(file))
    }

    async fn get_folder_files(&self, folder: &str) -> Result<Vec<FileInfo>> {
        let folder_path = self.folder_path(folder);
        if !folder_path.exists() {
            return Ok(vec![]);
        }

        let mut files = Vec::new();
        let mut entries = tokio::fs::read_dir(&folder_path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let content = tokio::fs::read_to_string(&path).await?;
                if let Ok(file) = serde_json::from_str::<FileInfo>(&content) {
                    files.push(file);
                }
            }
        }

        Ok(files)
    }

    async fn update_file(&self, folder: &str, info: FileInfo) -> Result<()> {
        let folder_path = self.folder_path(folder);
        tokio::fs::create_dir_all(&folder_path).await?;

        let path = self.file_path(folder, &info.name);
        let content = serde_json::to_string_pretty(&info)
            .map_err(|e| SyncError::database(format!("Failed to serialize file info: {}", e)))?;

        tokio::fs::write(&path, content).await?;

        let mut cache = self.cache.entry(folder.to_string()).or_default();
        let is_new = if let Some(idx) = cache.iter().position(|f| f.name == info.name) {
            cache[idx] = info;
            false
        } else {
            cache.push(info);
            true
        };
        if is_new {
            let size = self.cache_size.fetch_add(1, Ordering::Relaxed) + 1;
            if size > self.cache_cap {
                self.evict_one_folder();
            }
        }

        Ok(())
    }

    async fn update_files(&self, folder: &str, files: Vec<FileInfo>) -> Result<()> {
        let mut set = tokio::task::JoinSet::new();
        for file in files {
            let this = self.clone();
            let folder = folder.to_string();
            set.spawn(async move { this.update_file(&folder, file).await });
        }
        while let Some(res) = set.join_next().await {
            res.map_err(|e| SyncError::database(format!("update_files task: {}", e)))??;
        }
        Ok(())
    }

    async fn delete_file(&self, folder: &str, name: &str) -> Result<()> {
        let path = self.file_path(folder, name);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }

        // Keep the cache size counter in sync when entries are removed.
        if let Some(mut cache) = self.cache.get_mut(folder) {
            let before = cache.len();
            cache.retain(|f| f.name != name);
            let after = cache.len();
            if after < before {
                self.cache_size.fetch_sub(before - after, Ordering::Relaxed);
            }
        }

        Ok(())
    }

    async fn has_file(&self, folder: &str, name: &str) -> Result<bool> {
        let result: Option<FileInfo> = self.get_file(folder, name).await?;
        Ok(result.is_some())
    }

    async fn get_folder(&self, folder_id: &str) -> Result<Option<Folder>> {
        let path = self.base_path.join(format!("folder_{}.json", folder_id));
        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let folder: Folder = serde_json::from_str(&content)
            .map_err(|e| SyncError::database(format!("Failed to parse folder config: {}", e)))?;

        Ok(Some(folder))
    }

    async fn update_folder(&self, folder: Folder) -> Result<()> {
        let path = self.base_path.join(format!("folder_{}.json", folder.id));
        tokio::fs::create_dir_all(&self.base_path).await?;

        let content = serde_json::to_string_pretty(&folder)
            .map_err(|e| SyncError::database(format!("Failed to serialize folder: {}", e)))?;

        tokio::fs::write(&path, content).await?;
        Ok(())
    }

    async fn get_needed_files(&self, folder: &str, since: u64) -> Result<Vec<FileInfo>> {
        let files: Vec<FileInfo> = self.get_folder_files(folder).await?;
        Ok(files.into_iter().filter(|f| f.sequence > since).collect())
    }

    async fn check_globals(&self, folder: &str, name: &str) -> Result<Vec<FileInfo>> {
        // A full implementation would query the global database.
        if let Some(file) = self.get_file(folder, name).await? {
            Ok(vec![file])
        } else {
            Ok(vec![])
        }
    }

    async fn get_sequence(&self, folder: &str) -> Result<u64> {
        let seq_lock = self.get_seq_lock(folder);
        let _lock = seq_lock.lock().await;
        let path = self.base_path.join(format!("seq_{}", folder));
        if !path.exists() {
            return Ok(0);
        }

        let content = tokio::fs::read_to_string(&path).await?;
        content
            .parse::<u64>()
            .map_err(|e| SyncError::database(format!("Invalid sequence: {}", e)))
    }

    async fn increment_sequence(&self, folder: &str) -> Result<u64> {
        let seq_lock = self.get_seq_lock(folder);
        let _lock = seq_lock.lock().await;
        let path = self.base_path.join(format!("seq_{}", folder));
        let seq = if path.exists() {
            let content = tokio::fs::read_to_string(&path).await?;
            let current: u64 = content
                .parse()
                .map_err(|e| SyncError::database(format!("Invalid sequence: {}", e)))?;
            current + 1
        } else {
            1
        };
        tokio::fs::write(&path, seq.to_string()).await?;
        Ok(seq)
    }

    async fn reset_sequence(&self, folder: &str) -> Result<()> {
        let seq_lock = self.get_seq_lock(folder);
        let _lock = seq_lock.lock().await;
        let path = self.base_path.join(format!("seq_{}", folder));
        tokio::fs::write(&path, "0").await?;
        Ok(())
    }

    async fn get_folder_index_meta(&self, folder: &str) -> Result<Option<(IndexID, u64)>> {
        let path = self.base_path.join(format!("index_meta_{}.json", folder));
        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let meta: (IndexID, u64) = serde_json::from_str(&content)
            .map_err(|e| SyncError::database(format!("Failed to parse index meta: {}", e)))?;
        Ok(Some(meta))
    }

    async fn update_folder_index_meta(
        &self,
        folder: &str,
        index_id: IndexID,
        max_sequence: u64,
    ) -> Result<()> {
        let path = self.base_path.join(format!("index_meta_{}.json", folder));
        tokio::fs::create_dir_all(&self.base_path).await?;
        let content = serde_json::to_string_pretty(&(index_id, max_sequence))
            .map_err(|e| SyncError::database(format!("Failed to serialize index meta: {}", e)))?;
        tokio::fs::write(&path, content).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_database() {
        let db = MemoryDatabase::new();

        let file = FileInfo {
            name: "test.txt".to_string(),
            file_type: syncthing_core::types::FileType::File,
            size: 100,
            permissions: 0o644,
            modified_s: 1234567890,
            modified_ns: 0,
            version: syncthing_core::types::Vector::new(),
            sequence: 1,
            block_size: 128 * 1024,
            blocks: vec![],
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        };

        db.update_file("test-folder", file.clone()).await.unwrap();

        let retrieved = db.get_file("test-folder", "test.txt").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test.txt");
    }
}
