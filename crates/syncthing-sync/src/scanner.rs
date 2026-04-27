//! 文件夹扫描器
//! 
//! 实现定期扫描本地文件夹变更的功能

use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{EventPublisher, SyncEvent};
use crate::ignore::IgnoreMatcher;
use syncthing_core::types::{BlockInfo, FileInfo, FileType, Folder, Vector};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, error, info, trace};

/// 块大小（128KB，与 Syncthing 默认一致）
const DEFAULT_BLOCK_SIZE: i32 = 128 * 1024;

/// 文件夹扫描器
pub struct Scanner {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
}

impl Scanner {
    /// 创建新的扫描器
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher) -> Self {
        Self { db, events }
    }

    /// 扫描单个文件夹
    pub async fn scan_folder(&self, folder: &Folder) -> Result<Vec<FileInfo>> {
        self.scan_folder_at(folder, None).await
    }

    /// 扫描文件夹中的子目录
    pub async fn scan_folder_sub(&self, folder: &Folder, sub: &str) -> Result<Vec<FileInfo>> {
        self.scan_folder_at(folder, Some(sub)).await
    }

    async fn scan_folder_at(&self, folder: &Folder, sub: Option<&str>) -> Result<Vec<FileInfo>> {
        let path = Path::new(&folder.path);
        let scan_root = match sub {
            Some(s) => path.join(s),
            None => path.to_path_buf(),
        };

        info!(folder_id = %folder.id, path = %scan_root.display(), "Starting folder scan");

        if !scan_root.exists() {
            return Err(SyncError::scan(folder.id.clone(), format!("Path does not exist: {}", scan_root.display())));
        }

        if !scan_root.is_dir() {
            return Err(SyncError::scan(folder.id.clone(), format!("Path is not a directory: {}", scan_root.display())));
        }

        let mut changed_files = Vec::new();
        let mut visited_paths = std::collections::HashSet::new();

        // 加载 .stignore（如果存在）—— 始终以 folder 根目录为基准
        let ignore_path = path.join(".stignore");
        let matcher = IgnoreMatcher::load(&ignore_path);

        // 递归扫描目录
        match self.scan_directory(&folder.id, path, &scan_root, &mut visited_paths, &matcher).await {
            Ok(files) => {
                // 仅全量扫描时检查已删除的文件
                if sub.is_none() {
                    let db_files = self.db.get_folder_files(&folder.id).await?;
                    for db_file in db_files {
                        let full_path = path.join(&db_file.name);
                        if !full_path.exists() && !db_file.is_deleted() {
                            // FIX: 检查是否正在下载中（临时文件存在）
                            let temp_path = full_path.with_extension(".syncthing.tmp");
                            if temp_path.exists() {
                                debug!(file = %db_file.name, "File is being downloaded, skipping deleted check");
                                continue;
                            }
                            debug!(file = %db_file.name, "File was deleted");
                            let mut deleted_info = db_file.clone();
                            deleted_info.deleted = Some(true);
                            deleted_info.blocks = vec![]; // BEP 协议要求 deleted 文件 block list 为空
                            deleted_info.size = 0;
                            deleted_info.sequence = self.db.increment_sequence(&folder.id).await?;
                            deleted_info.version.increment(1); // 使用设备ID 1作为本地设备
                            changed_files.push(deleted_info);
                        }
                    }
                }

                // 检查变更的文件
                for file_info in files {
                    match self.db.get_file(&folder.id, &file_info.name).await? {
                        Some(existing) => {
                            if self.has_file_changed(&existing, &file_info) {
                                debug!(file = %file_info.name, "File was modified");
                                let mut new_info = file_info;
                                new_info.sequence = self.db.increment_sequence(&folder.id).await?;
                                new_info.version = existing.version.clone();
                                new_info.version.increment(1);
                                changed_files.push(new_info);
                            }
                        }
                        None => {
                            debug!(file = %file_info.name, "New file found");
                            let mut new_info = file_info;
                            new_info.sequence = self.db.increment_sequence(&folder.id).await?;
                            new_info.version = Vector::new().with_counter(1, 1);
                            changed_files.push(new_info);
                        }
                    }
                }
            }
            Err(e) => {
                error!(folder_id = %folder.id, error = %e, "Scan failed");
                self.events.publish(SyncEvent::FolderScanFailed {
                    folder: folder.id.clone(),
                    error: e.to_string(),
                });
                return Err(e);
            }
        }

        // 更新数据库
        for file in &changed_files {
            let file_clone: syncthing_core::types::FileInfo = file.clone();
            self.db.update_file(&folder.id, file_clone).await?;
        }

        info!(
            folder_id = %folder.id,
            files_changed = changed_files.len(),
            "Folder scan completed"
        );

        self.events.publish(SyncEvent::FolderScanCompleted {
            folder: folder.id.clone(),
            files_changed: changed_files.len(),
        });

        Ok(changed_files)
    }

    /// 递归扫描目录
    #[async_recursion::async_recursion]
    async fn scan_directory(
        &self,
        folder_id: &str,
        base_path: &Path,
        current_path: &Path,
        visited: &mut std::collections::HashSet<std::path::PathBuf>,
        matcher: &IgnoreMatcher,
    ) -> Result<Vec<FileInfo>> {
        let mut files = Vec::new();
        
        let entries = std::fs::read_dir(current_path).map_err(|e| {
            SyncError::scan(folder_id.to_string(), format!("Failed to read directory: {}", e))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                SyncError::scan(folder_id.to_string(), format!("Failed to read entry: {}", e))
            })?;
            let path = entry.path();
            
            // 跳过隐藏文件和特殊文件
            if let Some(file_name) = path.file_name() {
                let name = file_name.to_string_lossy();
                if name.starts_with('.') || name.starts_with("~") || name.ends_with(".tmp") {
                    trace!(path = %path.display(), "Skipping hidden/temp file");
                    continue;
                }
                // 跳过 Syncthing 冲突文件
                if name.contains(".sync-conflict-") {
                    trace!(path = %path.display(), "Skipping conflict file");
                    continue;
                }
            }

            let metadata = entry.metadata().map_err(|e| {
                SyncError::scan(folder_id.to_string(), format!("Failed to get metadata: {}", e))
            })?;

            let relative_path = path.strip_prefix(base_path)
                .map_err(|e| SyncError::scan(folder_id.to_string(), format!("Path error: {}", e)))?
                .to_string_lossy()
                .replace('\\', "/");

            // 应用 .stignore 规则
            let is_dir = metadata.is_dir();
            if matcher.matches(&relative_path, is_dir) {
                trace!(path = %relative_path, "Ignoring path via .stignore");
                continue;
            }

            if visited.contains(&path) {
                continue;
            }
            visited.insert(path.clone());

            if is_dir {
                // 递归扫描子目录
                let sub_files = self.scan_directory(folder_id, base_path, &path, visited, matcher).await?;
                files.extend(sub_files);

                // 添加目录条目
                let modified = metadata.modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let modified_secs = modified.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let modified_nanos = modified.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as i32;

                files.push(FileInfo {
                    name: relative_path.clone(),
                    file_type: FileType::Directory,
                    size: 0,
                    permissions: 0o755,
                    modified_s: modified_secs,
                    modified_ns: modified_nanos,
                    version: Vector::new(),
                    sequence: 0,
                    block_size: 0,
                    blocks: vec![],
                    symlink_target: None,
                    deleted: None,
                });
            } else if metadata.is_file() {
                // 计算文件哈希和块信息
                let file_info = self.scan_file(&path, &relative_path, &metadata, folder_id).await?;
                files.push(file_info);
            } else if metadata.is_symlink() {
                // 处理符号链接
                let target = fs::read_link(&path).await.map_err(|e| {
                    SyncError::scan(folder_id.to_string(), format!("Failed to read symlink: {}", e))
                })?;

                let modified = metadata.modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let modified_secs = modified.duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;

                files.push(FileInfo {
                    name: relative_path,
                    file_type: FileType::Symlink,
                    size: 0,
                    permissions: 0o755,
                    modified_s: modified_secs,
                    modified_ns: 0,
                    version: Vector::new(),
                    sequence: 0,
                    block_size: 0,
                    blocks: vec![],
                    symlink_target: Some(target.to_string_lossy().to_string()),
                    deleted: None,
                });
            }
        }

        Ok(files)
    }

    /// 扫描单个文件
    async fn scan_file(
        &self,
        path: &Path,
        relative_path: &str,
        metadata: &std::fs::Metadata,
        folder_id: &str,
    ) -> Result<FileInfo> {
        let size = metadata.len() as i64;
        let modified = metadata.modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let modified_secs = modified.duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let modified_nanos = modified.duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as i32;

        let permissions = 0o644;

        // 计算块信息
        let mut blocks = Vec::new();
        if size > 0 {
            let block_size = DEFAULT_BLOCK_SIZE;
            let num_blocks = ((size + block_size as i64 - 1) / block_size as i64) as usize;
            
            // 对于大文件，异步读取并计算哈希
            let file = tokio::fs::File::open(path).await.map_err(|e| {
                SyncError::scan(folder_id.to_string(), format!("Failed to open file: {}", e))
            })?;

            let mut reader = tokio::io::BufReader::new(file);
            let mut buffer = vec![0u8; block_size as usize];
            let mut offset = 0i64;

            for i in 0..num_blocks {
                let bytes_read = tokio::io::AsyncReadExt::read(&mut reader, &mut buffer).await
                    .map_err(|e| SyncError::scan(folder_id.to_string(), format!("Failed to read file: {}", e)))?;
                
                if bytes_read == 0 {
                    break;
                }

                let mut hasher = Sha256::new();
                hasher.update(&buffer[..bytes_read]);
                let hash = hasher.finalize().to_vec();

                blocks.push(BlockInfo {
                    size: bytes_read as i32,
                    hash,
                    offset,
                });

                offset += bytes_read as i64;

                if i % 100 == 0 {
                    trace!(file = %relative_path, block = i, "Scanning block");
                }
            }
        }

        Ok(FileInfo {
            name: relative_path.to_string(),
            file_type: FileType::File,
            size,
            permissions,
            modified_s: modified_secs,
            modified_ns: modified_nanos,
            version: Vector::new(),
            sequence: 0,
            block_size: DEFAULT_BLOCK_SIZE,
            blocks,
            symlink_target: None,
            deleted: None,
        })
    }

    /// 检查文件是否变更
    fn has_file_changed(&self, old: &FileInfo, new: &FileInfo) -> bool {
        // 检查大小
        if old.size != new.size {
            return true;
        }

        // 检查修改时间
        if old.modified_s != new.modified_s || old.modified_ns != new.modified_ns {
            return true;
        }

        // 检查权限
        if old.permissions != new.permissions {
            return true;
        }

        // 检查块哈希（如果数量相同）
        if old.blocks.len() == new.blocks.len() {
            for (old_block, new_block) in old.blocks.iter().zip(new.blocks.iter()) {
                if old_block.hash != new_block.hash {
                    return true;
                }
            }
            false
        } else {
            true
        }
    }

    /// 快速扫描（仅检查修改时间）
    pub async fn quick_scan(&self, folder: &Folder) -> Result<Vec<FileInfo>> {
        debug!(folder_id = %folder.id, "Starting quick scan");
        
        let db_files: Vec<syncthing_core::types::FileInfo> = self.db.get_folder_files(&folder.id).await?;
        let mut changed = Vec::new();
        let base_path = Path::new(&folder.path);

        for db_file in db_files {
            if db_file.is_deleted() {
                continue;
            }

            let full_path = base_path.join(&db_file.name);
            match fs::metadata(&full_path).await {
                Ok(metadata) => {
                    let modified = metadata.modified()
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    let modified_secs = modified.duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    let modified_nanos = modified.duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos() as i32;

                    if db_file.modified_s != modified_secs || db_file.modified_ns != modified_nanos {
                        changed.push(db_file);
                    }
                }
                Err(_) => {
                    // 文件已被删除（但检查是否正在下载中）
                    let temp_path = full_path.with_extension(".syncthing.tmp");
                    if temp_path.exists() {
                        debug!(file = %db_file.name, "File is being downloaded, skipping deleted check");
                        continue;
                    }
                    if !db_file.is_deleted() {
                        changed.push(db_file);
                    }
                }
            }
        }

        debug!(folder_id = %folder.id, changed_count = changed.len(), "Quick scan completed");
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryDatabase;

    #[tokio::test]
    async fn test_scan_empty_folder() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let scanner = Scanner::new(db, events);

        // 创建临时目录
        let temp_dir = tempfile::tempdir().unwrap();
        let folder = Folder::new("test", temp_dir.path().to_str().unwrap());

        let result = scanner.scan_folder(&folder).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
