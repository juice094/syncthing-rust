//! 文件拉取器
//! 
//! 实现从远程设备下载文件的功能

use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{EventPublisher, ItemAction, SyncEvent};
use syncthing_core::types::{BlockInfo, FileInfo, Folder};
use bytes::Bytes;
use sha2::Digest;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, trace, warn};

/// 临时文件后缀
const TEMP_SUFFIX: &str = ".syncthing.tmp";

/// 块数据源 trait
#[async_trait::async_trait]
pub trait BlockSource: Send + Sync {
    async fn request_block(&self, folder: &str, file: &str, block: &BlockInfo) -> Result<Bytes>;
}

/// 文件拉取器
pub struct Puller {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    max_concurrent_downloads: usize,
    block_source: Option<Arc<dyn BlockSource>>,
}

impl Puller {
    /// 创建新的拉取器
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher) -> Self {
        Self {
            db,
            events,
            max_concurrent_downloads: 4,
            block_source: None,
        }
    }

    /// 设置最大并发下载数
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent_downloads = max;
        self
    }

    /// 设置块数据源
    pub fn with_block_source(mut self, source: Option<Arc<dyn BlockSource>>) -> Self {
        self.block_source = source;
        self
    }

    /// 拉取文件夹
    pub async fn pull_folder(&self, folder: &Folder, needed_files: Vec<FileInfo>) -> Result<PullStats> {
        info!(folder_id = %folder.id, file_count = needed_files.len(), "Starting folder pull");

        let mut stats = PullStats::default();
        let base_path = Path::new(&folder.path);

        // 确保目标目录存在
        fs::create_dir_all(&base_path).await.map_err(|e| {
            SyncError::pull(folder.path.clone(), format!("Failed to create directory: {}", e))
        })?;

        // 使用信号量限制并发
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent_downloads));
        let mut handles = Vec::new();

        for file_info in needed_files {
            let permit = semaphore.clone().acquire_owned().await.map_err(|e| {
                SyncError::pull(file_info.name.clone(), format!("Failed to acquire permit: {}", e))
            })?;

            let db = self.db.clone();
            let events = self.events.clone();
            let folder_id = folder.id.clone();
            let folder_path = base_path.to_path_buf();
            let block_source = self.block_source.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit; // 持有直到任务完成
                
                events.publish(SyncEvent::ItemStarted {
                    folder: folder_id.clone(),
                    item: file_info.name.clone(),
                    action: if file_info.is_deleted() {
                        ItemAction::Delete
                    } else {
                        ItemAction::Modify
                    },
                });

                let result = if file_info.is_deleted() {
                    Self::delete_file(&folder_path, &file_info, &*db, &folder_id).await
                } else {
                    Self::download_file(&folder_path, &file_info, &*db, &events, &folder_id, block_source).await
                };

                match &result {
                    Ok(_) => {
                        events.publish(SyncEvent::ItemFinished {
                            folder: folder_id,
                            item: file_info.name.clone(),
                            action: if file_info.is_deleted() {
                                ItemAction::Delete
                            } else {
                                ItemAction::Modify
                            },
                            error: None,
                        });
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        events.publish(SyncEvent::ItemFinished {
                            folder: folder_id,
                            item: file_info.name.clone(),
                            action: if file_info.is_deleted() {
                                ItemAction::Delete
                            } else {
                                ItemAction::Modify
                            },
                            error: Some(err_str),
                        });
                    }
                }

                result
            });

            handles.push(handle);
        }

        // 等待所有任务完成
        for handle in handles {
            match handle.await {
                Ok(Ok(_)) => {
                    stats.files_succeeded += 1;
                }
                Ok(Err(e)) => {
                    error!(error = %e, "File pull failed");
                    stats.files_failed += 1;
                }
                Err(e) => {
                    error!(error = %e, "Task join failed");
                    stats.files_failed += 1;
                }
            }
        }

        info!(
            folder_id = %folder.id,
            succeeded = stats.files_succeeded,
            failed = stats.files_failed,
            "Folder pull completed"
        );

        Ok(stats)
    }

    /// 下载单个文件
    async fn download_file(
        folder_path: &Path,
        file_info: &FileInfo,
        db: &dyn LocalDatabase,
        events: &EventPublisher,
        folder_id: &str,
        block_source: Option<Arc<dyn BlockSource>>,
    ) -> Result<()> {
        debug!(file = %file_info.name, size = file_info.size, "Downloading file");

        let file_path = folder_path.join(&file_info.name);
        let temp_path = file_path.with_extension(TEMP_SUFFIX);

        // 确保父目录存在
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SyncError::pull(file_info.name.clone(), format!("Failed to create parent directory: {}", e))
            })?;
        }

        // 创建临时文件
        let mut file = fs::File::create(&temp_path).await.map_err(|e| {
            SyncError::pull(file_info.name.clone(), format!("Failed to create temp file: {}", e))
        })?;

        let mut bytes_downloaded = 0u64;

        // 下载每个块
        for (idx, block) in file_info.blocks.iter().enumerate() {
            trace!(file = %file_info.name, block = idx, offset = block.offset, size = block.size, "Downloading block");

            let block_data = match &block_source {
                Some(source) => source.request_block(folder_id, &file_info.name, block).await?,
                None => return Err(SyncError::pull(file_info.name.clone(), "No block source configured".to_string())),
            };

            // 验证块哈希
            let hash = sha2::Sha256::digest(&block_data);
            if hash.as_slice() != block.hash.as_slice() {
                return Err(SyncError::ChecksumMismatch { offset: block.offset });
            }

            // 写入文件
            file.write_all(&block_data).await.map_err(|e| {
                SyncError::pull(file_info.name.clone(), format!("Failed to write block: {}", e))
            })?;

            bytes_downloaded += block_data.len() as u64;

            // 发布进度事件
            events.publish(SyncEvent::DownloadProgress {
                folder: folder_id.to_string(),
                file: file_info.name.clone(),
                bytes_done: bytes_downloaded,
                bytes_total: file_info.size as u64,
            });
        }

        // 刷新并关闭文件
        file.flush().await.map_err(|e| {
            SyncError::pull(file_info.name.clone(), format!("Failed to flush file: {}", e))
        })?;
        drop(file);

        // 设置文件权限（Unix）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(file_info.permissions);
            fs::set_permissions(&temp_path, perms).await.map_err(|e| {
                SyncError::pull(file_info.name.clone(), format!("Failed to set permissions: {}", e))
            })?;
        }

        // 重命名为最终文件名
        fs::rename(&temp_path, &file_path).await.map_err(|e| {
            SyncError::pull(file_info.name.clone(), format!("Failed to rename file: {}", e))
        })?;

        // 设置修改时间（精确到纳秒）
        let modified = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(file_info.modified_s as u64)
            + std::time::Duration::from_nanos(file_info.modified_ns as u64);

        let mtime = filetime::FileTime::from_system_time(modified);
        if let Err(e) = filetime::set_file_mtime(&file_path, mtime) {
            warn!(
                file = %file_info.name,
                error = %e,
                "Failed to set file modification time"
            );
        }

        // 更新数据库，标记文件已同步
        if let Err(e) = db.update_file(folder_id, file_info.clone()).await {
            warn!(file = %file_info.name, error = %e, "Failed to update database after download");
        }

        info!(file = %file_info.name, "File download completed");
        Ok(())
    }

    /// 删除文件
    async fn delete_file(
        folder_path: &Path,
        file_info: &FileInfo,
        db: &dyn LocalDatabase,
        folder_id: &str,
    ) -> Result<()> {
        debug!(file = %file_info.name, "Deleting file");

        let file_path = folder_path.join(&file_info.name);

        if file_path.exists() {
            if file_path.is_dir() {
                fs::remove_dir_all(&file_path).await.map_err(|e| {
                    SyncError::pull(file_info.name.clone(), format!("Failed to remove directory: {}", e))
                })?;
            } else {
                fs::remove_file(&file_path).await.map_err(|e| {
                    SyncError::pull(file_info.name.clone(), format!("Failed to remove file: {}", e))
                })?;
            }
            info!(file = %file_info.name, "File deleted");
        } else {
            warn!(file = %file_info.name, "File to delete not found");
        }

        // 更新数据库中的删除状态
        db.update_file(folder_id, file_info.clone()).await?;

        Ok(())
    }

    /// 检查文件是否需要下载
    pub async fn check_needed_files(&self, folder: &Folder) -> Result<Vec<FileInfo>> {
        let db_files: Vec<syncthing_core::types::FileInfo> = self.db.get_folder_files(&folder.id).await?;
        let base_path = Path::new(&folder.path);
        let mut needed = Vec::new();

        for file_info in db_files {
            if file_info.is_deleted() {
                // 检查本地文件是否还存在
                let file_path = base_path.join(&file_info.name);
                if file_path.exists() {
                    needed.push(file_info);
                }
            } else {
                // 检查本地文件是否需要更新
                let file_path = base_path.join(&file_info.name);
                if !file_path.exists() {
                    needed.push(file_info);
                } else {
                    // 可以添加更多检查，如大小、修改时间等
                    let metadata = fs::metadata(&file_path).await?;
                    if metadata.len() != file_info.size as u64 {
                        needed.push(file_info);
                    }
                }
            }
        }

        Ok(needed)
    }
}

/// 拉取统计
#[derive(Debug, Clone, Default)]
pub struct PullStats {
    pub files_succeeded: usize,
    pub files_failed: usize,
    pub bytes_transferred: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryDatabase;
    use crate::scanner::Scanner;
    use syncthing_core::types::BlockInfo;

    struct MockBlockSource {
        data: Bytes,
    }

    #[async_trait::async_trait]
    impl BlockSource for MockBlockSource {
        async fn request_block(&self, _folder: &str, _file: &str, _block: &BlockInfo) -> Result<Bytes> {
            Ok(self.data.clone())
        }
    }

    #[tokio::test]
    async fn test_puller_creation() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let puller = Puller::new(db, events);
        // 基本创建测试
        assert!(puller.block_source.is_none());
    }

    #[tokio::test]
    async fn test_download_file_with_mock_source() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        
        // 创建临时目录
        let temp_dir = tempfile::tempdir().unwrap();
        let folder_path = temp_dir.path().to_path_buf();
        
        // 准备测试数据
        let test_data = b"hello world";
        let hash = sha2::Sha256::digest(test_data);
        
        let file_info = FileInfo {
            name: "test.txt".to_string(),
            file_type: syncthing_core::types::FileType::File,
            size: test_data.len() as i64,
            permissions: 0o644,
            modified_s: 0,
            modified_ns: 0,
            version: syncthing_core::types::Vector::new(),
            sequence: 0,
            block_size: test_data.len() as i32,
            blocks: vec![BlockInfo {
                size: test_data.len() as i32,
                hash: hash.to_vec(),
                offset: 0,
            }],
            symlink_target: None,
            deleted: Some(false),
        };
        
        let mock_source = Arc::new(MockBlockSource {
            data: Bytes::from_static(test_data),
        });
        
        let result = Puller::download_file(
            &folder_path,
            &file_info,
            &*db,
            &events,
            "test-folder",
            Some(mock_source),
        ).await;
        
        assert!(result.is_ok());
        
        // 验证文件内容
        let file_path = folder_path.join("test.txt");
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "hello world");
    }

    /// 集成测试：模拟远程索引更新后，check_needed_files 能发现 needed 文件，
    /// 且 pull_folder 能成功下载。
    #[tokio::test]
    async fn test_check_needed_files_then_pull() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        
        // 创建临时目录作为 folder path
        let temp_dir = tempfile::tempdir().unwrap();
        let folder_path = temp_dir.path().to_path_buf();
        let folder = syncthing_core::types::Folder::new("test-folder", folder_path.to_str().unwrap());
        
        // 准备测试数据
        let test_data = b"pull test content";
        let hash = sha2::Sha256::digest(test_data);
        
        let file_info = FileInfo {
            name: "pull_test.txt".to_string(),
            file_type: syncthing_core::types::FileType::File,
            size: test_data.len() as i64,
            permissions: 0o644,
            modified_s: 0,
            modified_ns: 0,
            version: syncthing_core::types::Vector::new(),
            sequence: 1,
            block_size: test_data.len() as i32,
            blocks: vec![BlockInfo {
                size: test_data.len() as i32,
                hash: hash.to_vec(),
                offset: 0,
            }],
            symlink_target: None,
            deleted: Some(false),
        };
        
        // 模拟 index_handler 处理远程索引后更新 DB
        db.update_file(&folder.id, file_info.clone()).await.unwrap();
        
        // 创建 Puller + MockBlockSource
        let mock_source = Arc::new(MockBlockSource {
            data: Bytes::from_static(test_data),
        });
        let puller = Puller::new(db.clone(), events.clone())
            .with_block_source(Some(mock_source));
        
        // Step 1: check_needed_files 应该发现本地不存在的文件
        let needed = puller.check_needed_files(&folder).await.unwrap();
        assert_eq!(needed.len(), 1, "Should detect 1 needed file");
        assert_eq!(needed[0].name, "pull_test.txt");
        
        // Step 2: pull_folder 应该成功下载文件
        let stats = puller.pull_folder(&folder, needed).await.unwrap();
        assert_eq!(stats.files_succeeded, 1, "Should succeed pulling 1 file");
        assert_eq!(stats.files_failed, 0);
        
        // Step 3: 验证本地文件内容正确
        let file_path = folder_path.join("pull_test.txt");
        assert!(file_path.exists(), "File should exist after pull");
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "pull test content");
        
        // Step 4: 再次 check_needed_files，应该为空（文件已存在且大小匹配）
        let needed_after = puller.check_needed_files(&folder).await.unwrap();
        assert!(needed_after.is_empty(), "Should not need pull after file exists");
    }

    /// E2E 集成测试：模拟两节点通过 block_server 同步单文件
    /// - 节点 A 扫描本地文件生成索引
    /// - 节点 B 接收索引，通过 block_server 从节点 A 读取块，完成 pull
    #[tokio::test]
    async fn test_e2e_sync_single_file_via_block_server() {
        use crate::index_handler::IndexHandler;
        use std::path::PathBuf;
        use syncthing_core::types::{Folder, Index};

        // 节点 A：创建临时文件并扫描
        let temp_a = tempfile::tempdir().unwrap();
        let file_path = temp_a.path().join("test.txt");
        tokio::fs::write(&file_path, "hello sync world").await.unwrap();

        let db_a = MemoryDatabase::new();
        let events_a = EventPublisher::new(10);
        let scanner_a = Scanner::new(db_a.clone(), events_a.clone());
        let folder_a = Folder::new("test", temp_a.path().to_str().unwrap());
        let changed = scanner_a.scan_folder(&folder_a).await.unwrap();
        assert_eq!(changed.len(), 1, "Should detect 1 changed file");
        let file_info = changed.into_iter().next().unwrap();

        // 节点 B：准备接收同步
        let temp_b = tempfile::tempdir().unwrap();
        let db_b = MemoryDatabase::new();
        let events_b = EventPublisher::new(10);

        // BlockSource 通过 block_server 从节点 A 读取块数据
        struct LocalBlockSource {
            folder_root: PathBuf,
        }
        #[async_trait::async_trait]
        impl BlockSource for LocalBlockSource {
            async fn request_block(
                &self,
                _folder: &str,
                file: &str,
                block: &BlockInfo,
            ) -> Result<Bytes> {
                let req = bep_protocol::messages::Request {
                    id: 1,
                    folder: "test".to_string(),
                    name: file.to_string(),
                    offset: block.offset,
                    size: block.size,
                    hash: block.hash.clone(),
                    from_temporary: false,
                    block_no: 0,
                };
                let data = crate::block_server::serve_block_request(&self.folder_root, &req)
                    .await
                    .map_err(|e| SyncError::pull(file.to_string(), e.to_string()))?;
                Ok(Bytes::from(data))
            }
        }

        let block_source = Arc::new(LocalBlockSource {
            folder_root: temp_a.path().to_path_buf(),
        });
        let puller = Puller::new(db_b.clone(), events_b.clone())
            .with_block_source(Some(block_source));

        // 节点 B 的 index_handler 处理节点 A 的索引
        let index_handler = IndexHandler::new(db_b.clone(), events_b.clone());
        let folder_b = Folder::new("test", temp_b.path().to_str().unwrap());
        let device_a = syncthing_core::DeviceId::random();
        let index = Index {
            folder: "test".to_string(),
            files: vec![file_info],
        };
        let needed = index_handler
            .handle_index(&folder_b, device_a, index)
            .await
            .unwrap();

        // 执行 pull
        let stats = puller.pull_folder(&folder_b, needed).await.unwrap();
        assert_eq!(stats.files_succeeded, 1, "Should pull 1 file");
        assert_eq!(stats.files_failed, 0);

        // 验证节点 B 本地文件内容
        let dest = temp_b.path().join("test.txt");
        assert!(dest.exists(), "File should exist after pull");
        let content = tokio::fs::read_to_string(&dest).await.unwrap();
        assert_eq!(content, "hello sync world");

        // 验证数据库已更新
        let db_file = db_b.get_file("test", "test.txt").await.unwrap();
        assert!(db_file.is_some(), "File should be in database after pull");
    }
}
