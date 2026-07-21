//! 文件拉取器
//!
//! 实现从远程设备下载文件的功能

pub mod concurrency;
pub(crate) mod ops;
pub(crate) mod rename;
pub use concurrency::{ConcurrencyPolicy, RttTracker};
use ops::{content_hash, find_local_copy_source, recover_base_content};
use rename::rename_with_retry;
pub(crate) use rename::temp_path_for;

use crate::block_server::validate_remote_name;
use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{EventPublisher, ItemAction, SyncEvent};
use crate::merge::{is_mergeable_text, three_way_merge};
use bytes::Bytes;
use sha2::Digest;
use std::path::Path;
use std::sync::{Arc, RwLock};
use syncthing_core::types::{BlockInfo, FileInfo, FileInfoBase, FileType, Folder};
use syncthing_versioner::Versioner;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, trace, warn};

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
    concurrency_policy: RwLock<Option<Arc<ConcurrencyPolicy>>>,
    block_source: RwLock<Option<Arc<dyn BlockSource>>>,
    versioner: Option<Arc<dyn Versioner>>,
}

impl Puller {
    /// 创建新的拉取器
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher) -> Self {
        Self {
            db,
            events,
            max_concurrent_downloads: 2, // 保守默认 — 高延迟链路友好
            max_concurrent_blocks: 4,    // 减少并发避免 HEAD-of-line blocking
            concurrency_policy: RwLock::new(None),
            block_source: RwLock::new(None),
            versioner: None,
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

    /// 设置共享并发策略
    pub fn with_concurrency_policy(mut self, policy: Option<Arc<ConcurrencyPolicy>>) -> Self {
        self.concurrency_policy = RwLock::new(policy);
        self
    }

    /// 动态更新共享并发策略
    pub fn set_concurrency_policy(&self, policy: Option<Arc<ConcurrencyPolicy>>) {
        if let Ok(mut guard) = self.concurrency_policy.write() {
            *guard = policy;
        }
    }

    /// 设置块数据源
    pub fn with_block_source(mut self, source: Option<Arc<dyn BlockSource>>) -> Self {
        self.block_source = RwLock::new(source);
        self
    }

    /// 动态更新块数据源（允许 add_folder 之后补配，修复此前仅创建时读取的缺陷）
    pub fn set_block_source(&self, source: Option<Arc<dyn BlockSource>>) {
        if let Ok(mut guard) = self.block_source.write() {
            *guard = source;
        }
    }

    /// 测试辅助：是否已配置块数据源
    #[cfg(test)]
    pub(crate) fn block_source_present(&self) -> bool {
        self.block_source
            .read()
            .map(|g| g.is_some())
            .unwrap_or(false)
    }

    /// 设置版本管理器
    pub fn with_versioner(mut self, versioner: Option<Arc<dyn Versioner>>) -> Self {
        self.versioner = versioner;
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

        // 批量删除速率防护：单次 pull 的远程删除数超过配额时拒绝整个删除批次
        //（下载/修改不受影响）。防止陈旧/损坏的对端索引静默清空本地——
        // 2026-07-20 事故中 114 个文件经增量批次被删，全量索引阈值无法覆盖。
        let local_count = self
            .db
            .get_folder_files(&folder.id)
            .await?
            .iter()
            .filter(|f| !f.is_deleted())
            .count();
        let quota = syncthing_core::constants::PULL_DELETE_QUOTA_MIN.max(
            (local_count as f64 * syncthing_core::constants::PULL_DELETE_QUOTA_RATIO) as usize,
        );
        let delete_count = needed_files.iter().filter(|f| f.is_deleted()).count();
        let needed_files = if delete_count > quota {
            error!(
                folder_id = %folder.id,
                delete_count,
                quota,
                local_files = local_count,
                "SAFETY QUOTA: refusing remote deletion batch; peer index may be stale/corrupt. \
                 Investigate the peer before allowing these deletions."
            );
            needed_files
                .into_iter()
                .filter(|f| !f.is_deleted())
                .collect::<Vec<_>>()
        } else {
            needed_files
        };

        // 顺带清理过期回收站（每次 pull 一次，代价极低）
        Self::cleanup_sttrash(base_path).await;

        // 使用信号量限制并发：优先读取共享并发策略，否则使用固定默认值
        let policy = self.concurrency_policy.read().ok().and_then(|g| g.clone());
        let max_concurrent_downloads = policy
            .as_ref()
            .map(|p| p.downloads())
            .unwrap_or(self.max_concurrent_downloads);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent_downloads));
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
            let block_source = self.block_source.read().ok().and_then(|g| g.clone());
            let max_concurrent_blocks = policy
                .as_ref()
                .map(|p| p.blocks())
                .unwrap_or(self.max_concurrent_blocks);
            let versioner = self.versioner.clone();

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
                    match file_info.file_type {
                        FileType::Directory => {
                            Self::create_directory(&folder_path, &file_info).await
                        }
                        FileType::File => {
                            Self::download_file(
                                &folder_path,
                                &file_info,
                                &*db,
                                &events,
                                &folder_id,
                                block_source,
                                max_concurrent_blocks,
                                versioner.as_ref(),
                            )
                            .await
                        }
                        FileType::Symlink => {
                            warn!(file = %file_info.name, "Symlink sync not yet implemented, skipping");
                            Ok(())
                        }
                    }
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
        let mut stale_index_errors = 0usize;
        for handle in handles {
            match handle.await {
                Ok(Ok(_)) => {
                    stats.files_succeeded += 1;
                }
                Ok(Err(e)) => {
                    // ponytail: BEP error code 2 (NO_SUCH_FILE) 经桥接层格式化为字符串，
                    // 此处以子串统计；结构化错误码需改桥接层签名，收益不成比例。
                    if e.to_string().contains("error code 2") {
                        stale_index_errors += 1;
                    }
                    error!(error = %e, "File pull failed");
                    stats.files_failed += 1;
                }
                Err(e) => {
                    error!(error = %e, "Task join failed");
                    stats.files_failed += 1;
                }
            }
        }

        if stale_index_errors > syncthing_core::constants::STALE_INDEX_WARN_THRESHOLD {
            warn!(
                folder_id = %folder.id,
                count = stale_index_errors,
                "Many NO_SUCH_FILE (error code 2) responses: peer index may be stale/corrupt. \
                 Recommend a full rescan on the peer before further sync."
            );
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
    #[allow(clippy::too_many_arguments)]
    async fn download_file(
        folder_path: &Path,
        file_info: &FileInfo,
        db: &dyn LocalDatabase,
        events: &EventPublisher,
        folder_id: &str,
        block_source: Option<Arc<dyn BlockSource>>,
        max_concurrent_blocks: usize,
        versioner: Option<&Arc<dyn Versioner>>,
    ) -> Result<()> {
        // SAFETY: 防御路径穿越 — 拒绝来自远程对端的恶意文件名
        validate_remote_name(&file_info.name).map_err(|e| {
            SyncError::pull(
                file_info.name.clone(),
                format!("Invalid remote file name (path traversal rejected): {}", e),
            )
        })?;

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

        // P1: 重命名优化——如果本地有相同块哈希的文件，直接复制
        if let Some(source_path) =
            find_local_copy_source(folder_path, file_info, db, folder_id).await?
        {
            info!(file = %file_info.name, source = %source_path.display(), "Copying from local file (rename optimization)");
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    SyncError::pull(
                        file_info.name.clone(),
                        format!("Failed to create parent directory: {}", e),
                    )
                })?;
            }
            fs::copy(&source_path, &temp_path).await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to copy from local source: {}", e),
                )
            })?;
            // 在覆盖前存档旧版本
            if let Some(v) = versioner {
                if file_path.exists() {
                    let _ = v.archive(&file_path).await;
                }
            }
            rename_with_retry(&temp_path, &file_path, &file_info.name).await?;
            // 设置修改时间
            let modified = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(file_info.modified_s as u64)
                + std::time::Duration::from_nanos(file_info.modified_ns as u64);
            let mtime = filetime::FileTime::from_system_time(modified);
            if let Err(e) = filetime::set_file_mtime(&file_path, mtime) {
                warn!(file = %file_info.name, error = %e, "Failed to set file modification time");
            }
            if let Err(e) = db.update_file(folder_id, file_info.clone()).await {
                warn!(file = %file_info.name, error = %e, "Failed to update database after local copy");
            }
            return Ok(());
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

        // 在覆盖前尝试三路合并（文本文件且本地 DB 有 base_version 时）
        let mut merged_info = file_info.clone();
        let local_info = db.get_file(folder_id, &file_info.name).await.ok().flatten();
        debug!(
            file = %file_info.name,
            exists = file_path.exists(),
            has_base = local_info.as_ref().and_then(|i| i.base_version.as_ref()).is_some(),
            has_versioner = versioner.is_some(),
            "Checking merge preconditions"
        );
        if file_path.exists() {
            if let Some(base) = local_info.as_ref().and_then(|i| i.base_version.as_ref()) {
                if is_mergeable_text(&file_info.name) {
                    if let (Ok(local_content), Ok(remote_content)) = (
                        fs::read_to_string(&file_path).await,
                        fs::read_to_string(&temp_path).await,
                    ) {
                        if let Some(v) = versioner {
                            debug!("Attempting base content recovery for three-way merge");
                            if let Some(base_content) =
                                recover_base_content(&**v, &file_path, base).await
                            {
                                debug!("Base recovered, performing three-way merge");
                                let merged = three_way_merge(
                                    &base_content,
                                    &local_content,
                                    &remote_content,
                                    &file_info.name,
                                );
                                debug!(
                                    file = %file_info.name,
                                    merged_len = merged.content.len(),
                                    conflicts = merged.conflict_count,
                                    "Three-way merge result computed"
                                );
                                if let Err(e) = fs::write(&temp_path, &merged.content).await {
                                    warn!(file = %file_info.name, error = %e, "Failed to write merged content");
                                } else {
                                    info!(
                                        file = %file_info.name,
                                        conflicts = merged.conflict_count,
                                        has_conflicts = merged.has_conflicts,
                                        "Three-way merge completed"
                                    );
                                    // 更新 base_version 为合并结果
                                    let hash = content_hash(merged.content.as_bytes());
                                    merged_info.base_version = Some(FileInfoBase {
                                        size: merged.content.len() as i64,
                                        modified_s: file_info.modified_s,
                                        modified_ns: file_info.modified_ns,
                                        blocks_hash: None,
                                        content_hash: Some(hash),
                                    });
                                }
                            } else {
                                warn!(file = %file_info.name, "Failed to recover base content for merge");
                            }
                        }
                    }
                }
            }
        }

        // 在覆盖前存档旧版本
        if let Some(v) = versioner {
            if file_path.exists() {
                if let Err(e) = v.archive(&file_path).await {
                    warn!(file = %file_info.name, error = %e, "Failed to archive old version");
                }
            }
        }

        // 重命名为最终文件名
        rename_with_retry(&temp_path, &file_path, &file_info.name).await?;

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
        if let Err(e) = db.update_file(folder_id, merged_info).await {
            warn!(file = %file_info.name, error = %e, "Failed to update database after download");
        }

        info!(file = %file_info.name, "File download completed");
        Ok(())
    }

    /// 创建目录
    async fn create_directory(folder_path: &Path, file_info: &FileInfo) -> Result<()> {
        // SAFETY: 防御路径穿越
        validate_remote_name(&file_info.name).map_err(|e| {
            SyncError::pull(
                file_info.name.clone(),
                format!(
                    "Invalid remote directory name (path traversal rejected): {}",
                    e
                ),
            )
        })?;

        let dir_path = folder_path.join(&file_info.name);

        if dir_path.exists() {
            if !dir_path.is_dir() {
                return Err(SyncError::pull(
                    file_info.name.clone(),
                    format!("Path exists but is not a directory: {}", dir_path.display()),
                ));
            }
        } else {
            fs::create_dir_all(&dir_path).await.map_err(|e| {
                SyncError::pull(
                    file_info.name.clone(),
                    format!("Failed to create directory: {}", e),
                )
            })?;
            info!(dir = %file_info.name, "Directory created");
        }

        Ok(())
    }

    /// 删除文件
    async fn delete_file(
        folder_path: &Path,
        file_info: &FileInfo,
        db: &dyn LocalDatabase,
        folder_id: &str,
    ) -> Result<()> {
        // SAFETY: 防御路径穿越
        validate_remote_name(&file_info.name).map_err(|e| {
            SyncError::pull(
                file_info.name.clone(),
                format!(
                    "Invalid remote file name for deletion (path traversal rejected): {}",
                    e
                ),
            )
        })?;

        debug!(file = %file_info.name, "Deleting file");

        let file_path = folder_path.join(&file_info.name);

        if file_path.exists() {
            // 软删除：远程驱动的删除先隔离到 .sttrash，避免对端索引损坏时
            // 本地数据不可恢复（2026-07-20 事故 versioning: null 零兜底）。
            match Self::move_to_trash(&file_path, folder_path, &file_info.name).await {
                Ok(trash_path) => {
                    info!(
                        file = %file_info.name,
                        trash = %trash_path.display(),
                        "File moved to .sttrash (remote deletion applied)"
                    );
                }
                Err(e) => {
                    return Err(SyncError::pull(
                        file_info.name.clone(),
                        format!("Failed to quarantine file to .sttrash: {}", e),
                    ));
                }
            }
        } else {
            warn!(file = %file_info.name, "File to delete not found");
        }

        // 更新数据库中的删除状态
        db.update_file(folder_id, file_info.clone()).await?;

        Ok(())
    }

    /// 将目标移入 `<folder>/.sttrash/<YYYYMMDD>/<相对路径>`，重名时追加时间戳。
    /// 返回回收站中的最终路径。
    async fn move_to_trash(
        file_path: &Path,
        folder_path: &Path,
        rel_name: &str,
    ) -> Result<std::path::PathBuf> {
        let date = chrono::Utc::now().format("%Y%m%d").to_string();
        let trash_dir = folder_path.join(".sttrash").join(&date);
        let mut dest = trash_dir.join(rel_name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SyncError::pull(
                    rel_name.to_string(),
                    format!("Failed to create .sttrash dir: {}", e),
                )
            })?;
        }
        if dest.exists() {
            let ts = chrono::Utc::now().format("%H%M%S").to_string();
            let file_name = dest
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            dest.set_file_name(format!("{}.{}", file_name, ts));
        }
        fs::rename(file_path, &dest).await.map_err(|e| {
            SyncError::pull(
                rel_name.to_string(),
                format!("Failed to move to .sttrash: {}", e),
            )
        })?;
        Ok(dest)
    }

    /// 清理 .sttrash 中超过保留期（7 天）的日期目录。
    /// ponytail: 按目录名日期判断，不递归检查内容；清理失败仅告警。
    async fn cleanup_sttrash(folder_path: &Path) {
        let trash_root = folder_path.join(".sttrash");
        let mut entries = match fs::read_dir(&trash_root).await {
            Ok(e) => e,
            Err(_) => return, // 无回收站，正常路径
        };
        let cutoff = chrono::Utc::now() - chrono::Duration::days(7);
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(date) = chrono::NaiveDate::parse_from_str(&name, "%Y%m%d") else {
                continue;
            };
            let Some(datetime) = date.and_hms_opt(0, 0, 0) else {
                continue;
            };
            if chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(datetime, chrono::Utc)
                < cutoff
            {
                info!(dir = %entry.path().display(), "Removing expired .sttrash directory");
                if let Err(e) = fs::remove_dir_all(entry.path()).await {
                    warn!(dir = %entry.path().display(), "Failed to remove expired .sttrash: {}", e);
                }
            }
        }
    }

    /// 检查文件是否需要下载
    pub async fn check_needed_files(&self, folder: &Folder) -> Result<Vec<FileInfo>> {
        let db_files: Vec<syncthing_core::types::FileInfo> =
            self.db.get_folder_files(&folder.id).await?;
        let base_path = Path::new(&folder.path);
        let mut needed = Vec::new();
        let mut dirs_unchanged = 0u64;

        for file_info in db_files {
            if file_info.is_deleted() {
                // 检查本地文件是否还存在
                let file_path = base_path.join(&file_info.name);
                if file_path.exists() {
                    needed.push(file_info);
                }
            } else {
                // 检查本地文件/目录是否需要更新
                let file_path = base_path.join(&file_info.name);
                if !file_path.exists() {
                    needed.push(file_info);
                } else if file_info.file_type == FileType::Directory {
                    // 目录只需存在即可，不检查大小/修改时间
                    dirs_unchanged += 1;
                } else {
                    // 可以添加更多检查，如大小、修改时间等
                    let metadata = fs::metadata(&file_path).await?;
                    if metadata.len() != file_info.size as u64 {
                        needed.push(file_info);
                    }
                }
            }
        }

        if dirs_unchanged > 0 {
            trace!(folder_id = %folder.id, count = dirs_unchanged, "Existing directories need no update");
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
