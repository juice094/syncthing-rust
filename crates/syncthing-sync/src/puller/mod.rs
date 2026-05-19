//! 文件拉取器
//!
//! 实现从远程设备下载文件的功能

use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{EventPublisher, ItemAction, SyncEvent};
use bytes::Bytes;
use sha2::Digest;
use std::path::Path;
use std::sync::Arc;
use syncthing_core::types::{BlockInfo, FileInfo, Folder};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, trace, warn};

/// 生成与 block_server 对齐的临时文件路径
/// 格式: `.syncthing.{filename}.tmp`
fn temp_path_for(file_path: &Path) -> std::path::PathBuf {
    let parent = file_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    parent.join(format!(".syncthing.{}.tmp", file_name))
}

/// 块数据源 trait
#[async_trait::async_trait]
pub trait BlockSource: Send + Sync {
    async fn request_block(
        &self,
        folder: &str,
        file: &str,
        block: &BlockInfo,
        block_no: usize,
    ) -> Result<Bytes>;
}

/// 文件拉取器
pub struct Puller {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    max_concurrent_downloads: usize,
    max_concurrent_blocks: usize,
    block_source: Option<Arc<dyn BlockSource>>,
}

impl Puller {
    /// 创建新的拉取器
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher) -> Self {
        Self {
            db,
            events,
            max_concurrent_downloads: 4,
            max_concurrent_blocks: 16,
            block_source: None,
        }
    }

    /// 设置最大并发下载数（文件级）
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent_downloads = max;
        self
    }

    /// 设置单文件内最大并发块请求数
    pub fn with_max_concurrent_blocks(mut self, max: usize) -> Self {
        self.max_concurrent_blocks = max;
        self
    }

    /// 设置块数据源
    pub fn with_block_source(mut self, source: Option<Arc<dyn BlockSource>>) -> Self {
        self.block_source = source;
        self
    }

    /// 拉取文件夹
    pub async fn pull_folder(
        &self,
        folder: &Folder,
        needed_files: Vec<FileInfo>,
    ) -> Result<PullStats> {
        info!(folder_id = %folder.id, file_count = needed_files.len(), "Starting folder pull");

        let mut stats = PullStats::default();
        let base_path = Path::new(&folder.path);

        // 确保目标目录存在
        fs::create_dir_all(&base_path).await.map_err(|e| {
            SyncError::pull(
                folder.path.clone(),
                format!("Failed to create directory: {}", e),
            )
        })?;

        // 使用信号量限制并发
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_concurrent_downloads));
        let mut handles = Vec::new();

        for file_info in needed_files {
            let permit = semaphore.clone().acquire_owned().await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to acquire permit: {}", e),
                )
            })?;

            let db = self.db.clone();
            let events = self.events.clone();
            let folder_id = folder.id.clone();
            let folder_path = base_path.to_path_buf();
            let block_source = self.block_source.clone();
            let max_concurrent_blocks = self.max_concurrent_blocks;

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
                    Self::download_file(
                        &folder_path,
                        &file_info,
                        &*db,
                        &events,
                        &folder_id,
                        block_source,
                        max_concurrent_blocks,
                    )
                    .await
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
        max_concurrent_blocks: usize,
    ) -> Result<()> {
        debug!(file = %file_info.name, size = file_info.size, blocks = file_info.blocks.len(), max_concurrent = max_concurrent_blocks, "Downloading file");

        let file_path = folder_path.join(&file_info.name);
        let temp_path = temp_path_for(&file_path);

        // 辅助函数：下载失败时清理临时文件
        async fn cleanup_temp(path: &Path) {
            if path.exists() {
                if let Err(e) = fs::remove_file(path).await {
                    warn!(path = %path.display(), error = %e, "Failed to cleanup temp file");
                } else {
                    debug!(path = %path.display(), "Cleaned up temp file after failed download");
                }
            }
        }

        // 确保父目录存在
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to create parent directory: {}", e),
                )
            })?;
        }

        // 创建临时文件
        let mut file = fs::File::create(&temp_path).await.map_err(|e| {
            SyncError::pull(
                file_info.name.clone(),
                format!("Failed to create temp file: {}", e),
            )
        })?;

        let mut bytes_downloaded = 0u64;

        // 块级并发：使用信号量限制并发请求数，JoinHandle 按顺序 await 保证写入顺序
        let block_semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_blocks));
        let mut handles = Vec::with_capacity(file_info.blocks.len());

        for (idx, block) in file_info.blocks.iter().enumerate() {
            let permit = block_semaphore.clone().acquire_owned().await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to acquire block permit: {}", e),
                )
            })?;

            let source = match &block_source {
                Some(s) => s.clone(),
                None => {
                    cleanup_temp(&temp_path).await;
                    return Err(SyncError::pull(
                        file_info.name.clone(),
                        "No block source configured".to_string(),
                    ));
                }
            };

            let folder_id = folder_id.to_string();
            let file_name = file_info.name.clone();
            let block = block.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;
                let data = source
                    .request_block(&folder_id, &file_name, &block, idx)
                    .await?;
                Ok::<_, SyncError>((idx, block.offset, data, block.hash))
            });
            handles.push(handle);
        }

        // 按顺序 await 并写入，保持文件块顺序
        for handle in handles {
            let (idx, offset, block_data, expected_hash) = match handle.await {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => {
                    error!(file = %file_info.name, error = %e, "Block download failed");
                    cleanup_temp(&temp_path).await;
                    return Err(e);
                }
                Err(e) => {
                    error!(file = %file_info.name, error = %e, "Block download task panicked");
                    cleanup_temp(&temp_path).await;
                    return Err(SyncError::pull(
                        file_info.name.clone(),
                        format!("Block download task panicked: {}", e),
                    ));
                }
            };

            trace!(file = %file_info.name, block = idx, offset = offset, size = block_data.len(), "Writing block");

            // 验证块哈希
            let hash = sha2::Sha256::digest(&block_data);
            if hash.as_slice() != expected_hash.as_slice() {
                cleanup_temp(&temp_path).await;
                return Err(SyncError::ChecksumMismatch { offset });
            }

            // 写入文件
            if let Err(e) = file.write_all(&block_data).await {
                cleanup_temp(&temp_path).await;
                return Err(SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to write block: {}", e),
                ));
            }

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
        if let Err(e) = file.flush().await {
            cleanup_temp(&temp_path).await;
            return Err(SyncError::pull(
                file_info.name.clone(),
                format!("Failed to flush file: {}", e),
            ));
        }
        drop(file);

        // 设置文件权限（Unix）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(file_info.permissions);
            if let Err(e) = fs::set_permissions(&temp_path, perms).await {
                cleanup_temp(&temp_path).await;
                return Err(SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to set permissions: {}", e),
                ));
            }
        }

        // 重命名为最终文件名
        if let Err(e) = fs::rename(&temp_path, &file_path).await {
            cleanup_temp(&temp_path).await;
            return Err(SyncError::pull(
                file_info.name.clone(),
                format!("Failed to rename file: {}", e),
            ));
        }

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
                    SyncError::pull(
                        file_info.name.clone(),
                        format!("Failed to remove directory: {}", e),
                    )
                })?;
            } else {
                fs::remove_file(&file_path).await.map_err(|e| {
                    SyncError::pull(
                        file_info.name.clone(),
                        format!("Failed to remove file: {}", e),
                    )
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
        let db_files: Vec<syncthing_core::types::FileInfo> =
            self.db.get_folder_files(&folder.id).await?;
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
mod tests;
