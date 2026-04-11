//! 本地数据库接口
//! 
//! 提供文件元数据的本地存储和查询

use crate::error::{Result, SyncError};
use syncthing_core::types::{FileInfo, Folder, IndexID};
use dashmap::DashMap;
use std::path::Path;
use std::sync::Arc;

/// 本地数据库 trait
#[async_trait::async_trait]
pub trait LocalDatabase: Send + Sync {
    /// 获取文件夹中的文件
    async fn get_file(&self, folder: &str, name: &str) -> Result<Option<FileInfo>>;

    /// 获取文件夹中的所有文件
    async fn get_folder_files(&self, folder: &str) -> Result<Vec<FileInfo>>;

    /// 更新文件信息
    async fn update_file(&self, folder: &str, info: FileInfo) -> Result<()>;

    /// 批量更新文件
    async fn update_files(&self, folder: &str, files: Vec<FileInfo>) -> Result<()>;

    /// 删除文件记录
    async fn delete_file(&self, folder: &str, name: &str) -> Result<()>;

    /// 检查文件是否存在
    async fn has_file(&self, folder: &str, name: &str) -> Result<bool>;

    /// 获取文件夹配置
    async fn get_folder(&self, folder_id: &str) -> Result<Option<Folder>>;

    /// 更新文件夹配置
    async fn update_folder(&self, folder: Folder) -> Result<()>;

    /// 获取需要同步的文件列表
    async fn get_needed_files(&self, folder: &str, since: u64) -> Result<Vec<FileInfo>>;

    /// 获取全局版本（检查全局状态）
    async fn check_globals(&self, folder: &str, name: &str) -> Result<Vec<FileInfo>>;

    /// 记录序列号
    async fn get_sequence(&self, folder: &str) -> Result<u64>;

    /// 增加序列号
    async fn increment_sequence(&self, folder: &str) -> Result<u64>;

    /// 获取文件夹索引元数据
    async fn get_folder_index_meta(&self, folder: &str) -> Result<Option<(IndexID, u64)>>;

    /// 更新文件夹索引元数据
    async fn update_folder_index_meta(&self, folder: &str, index_id: IndexID, max_sequence: u64) -> Result<()>;
}

/// 内存数据库实现（用于测试）
#[derive(Debug, Default)]
pub struct MemoryDatabase {
    files: DashMap<String, Vec<FileInfo>>,
    folders: DashMap<String, Folder>,
    sequences: DashMap<String, u64>,
    index_metas: DashMap<String, (IndexID, u64)>,
}

impl MemoryDatabase {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait::async_trait]
impl LocalDatabase for MemoryDatabase {
    async fn get_file(&self, folder: &str, name: &str) -> Result<Option<FileInfo>> {
        Ok(self
            .files
            .get(folder)
            .and_then(|files: dashmap::mapref::one::Ref<'_, String, Vec<FileInfo>>| files.iter().find(|f| f.name == name).cloned()))
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
        Ok(self.folders.get(folder_id).map(|f: dashmap::mapref::one::Ref<'_, String, Folder>| f.clone()))
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
        // 简化的全局检查：返回该文件的所有版本记录
        // 实际实现应该从全局数据库查询
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

    async fn get_folder_index_meta(&self, folder: &str) -> Result<Option<(IndexID, u64)>> {
        Ok(self.index_metas.get(folder).map(|m| *m))
    }

    async fn update_folder_index_meta(&self, folder: &str, index_id: IndexID, max_sequence: u64) -> Result<()> {
        self.index_metas.insert(folder.to_string(), (index_id, max_sequence));
        Ok(())
    }
}

/// 文件系统数据库（实际持久化实现）
pub struct FileSystemDatabase {
    base_path: std::path::PathBuf,
    cache: DashMap<String, Vec<FileInfo>>,
}

impl FileSystemDatabase {
    pub fn new(base_path: impl AsRef<Path>) -> Arc<Self> {
        Arc::new(Self {
            base_path: base_path.as_ref().to_path_buf(),
            cache: DashMap::new(),
        })
    }

    fn folder_path(&self, folder: &str) -> std::path::PathBuf {
        self.base_path.join(folder)
    }

    fn file_path(&self, folder: &str, name: &str) -> std::path::PathBuf {
        self.folder_path(folder).join(format!("{}.json", name.replace('/', "_")))
    }
}

#[async_trait::async_trait]
impl LocalDatabase for FileSystemDatabase {
    async fn get_file(&self, folder: &str, name: &str) -> Result<Option<FileInfo>> {
        // 先检查缓存
        if let Some(files) = self.cache.get(folder) {
            if let Some(file) = files.iter().find(|f| f.name == name) {
                return Ok(Some(file.clone()));
            }
        }

        // 从磁盘加载
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
        
        // 更新缓存
        let mut cache = self.cache.entry(folder.to_string()).or_default();
        if let Some(idx) = cache.iter().position(|f| f.name == info.name) {
            cache[idx] = info;
        } else {
            cache.push(info);
        }
        
        Ok(())
    }

    async fn update_files(&self, folder: &str, files: Vec<FileInfo>) -> Result<()> {
        for file in files {
            self.update_file(folder, file).await?;
        }
        Ok(())
    }

    async fn delete_file(&self, folder: &str, name: &str) -> Result<()> {
        let path = self.file_path(folder, name);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }

        // 更新缓存
        if let Some(mut cache) = self.cache.get_mut(folder) {
            cache.retain(|f| f.name != name);
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
        // 实际实现应该从全局数据库查询
        if let Some(file) = self.get_file(folder, name).await? {
            Ok(vec![file])
        } else {
            Ok(vec![])
        }
    }

    async fn get_sequence(&self, folder: &str) -> Result<u64> {
        let path = self.base_path.join(format!("seq_{}", folder));
        if !path.exists() {
            return Ok(0);
        }

        let content = tokio::fs::read_to_string(&path).await?;
        content.parse::<u64>().map_err(|e| SyncError::database(format!("Invalid sequence: {}", e)))
    }

    async fn increment_sequence(&self, folder: &str) -> Result<u64> {
        let path = self.base_path.join(format!("seq_{}", folder));
        let seq = self.get_sequence(folder).await? + 1;
        tokio::fs::write(&path, seq.to_string()).await?;
        Ok(seq)
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

    async fn update_folder_index_meta(&self, folder: &str, index_id: IndexID, max_sequence: u64) -> Result<()> {
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
        };

        db.update_file("test-folder", file.clone()).await.unwrap();
        
        let retrieved = db.get_file("test-folder", "test.txt").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test.txt");
    }
}
