//! 文件拉取器
//!
//! 实现从远程设备下载文件的功能

pub mod concurrency;
pub use concurrency::{ConcurrencyPolicy, RttTracker};

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

/// 重试配置
const RENAME_RETRY_BASE_DELAY_MS: u64 = 1000;
const RENAME_RETRY_MAX_ATTEMPTS: u32 = 5;

/// Windows-aware 原子重命名，带指数退避重试
///
/// 处理 Windows 上目标文件被其他进程锁定（杀毒软件、编辑器、桌面搜索等）
/// 导致的 `ERROR_SHARING_VIOLATION` (32) 和 `ERROR_ACCESS_DENIED` (5)。
///
/// 策略:
/// 1. 直接 rename
/// 2. 失败 → remove(target) → rename
/// 3. 仍失败 → 指数退避重试 (1s/2s/4s/8s)，最多 5 次
/// 4. 最终失败 → 保留 .tmp，返回错误（下次 pull 周期再试）
async fn rename_with_retry(temp_path: &Path, file_path: &Path, file_name: &str) -> Result<()> {
    match fs::rename(temp_path, file_path).await {
        Ok(()) => return Ok(()),
        Err(e) => {
            warn!(
                file = %file_name,
                error = %e,
                raw_os_error = ?e.raw_os_error(),
                "Initial rename failed, trying remove+rename fallback"
            );
        }
    }

    // Fallback: 先删目标再重命名
    if file_path.exists() {
        if let Err(e) = fs::remove_file(file_path).await {
            warn!(
                file = %file_name,
                error = %e,
                "Failed to remove target file before rename retry"
            );
        }
    }

    match fs::rename(temp_path, file_path).await {
        Ok(()) => {
            warn!(
                file = %file_name,
                "Rename succeeded after remove fallback"
            );
            return Ok(());
        }
        Err(e) => {
            warn!(
                file = %file_name,
                error = %e,
                raw_os_error = ?e.raw_os_error(),
                "Rename failed after remove fallback, starting exponential backoff"
            );
        }
    }

    // 指数退避重试
    let mut delay_ms = RENAME_RETRY_BASE_DELAY_MS;
    for attempt in 1..=RENAME_RETRY_MAX_ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

        // 每次重试前尝试删除目标（可能已解锁）
        if file_path.exists() {
            let _ = fs::remove_file(file_path).await;
        }

        match fs::rename(temp_path, file_path).await {
            Ok(()) => {
                warn!(
                    file = %file_name,
                    attempt = attempt,
                    "Rename succeeded after retry"
                );
                return Ok(());
            }
            Err(e) => {
                warn!(
                    file = %file_name,
                    attempt = attempt,
                    delay_ms = delay_ms,
                    error = %e,
                    raw_os_error = ?e.raw_os_error(),
                    "Rename retry failed"
                );
            }
        }

        delay_ms *= 2;
    }

    // 所有重试耗尽 —— 保留 .tmp，让下一次 pull 周期重试
    error!(
        file = %file_name,
        temp = %temp_path.display(),
        target = %file_path.display(),
        "Rename exhausted all retries, preserving temp file for next pull cycle"
    );

    Err(SyncError::pull(
        file_name.to_string(),
        format!(
            "Failed to rename file after {} retries (temp preserved at {})",
            RENAME_RETRY_MAX_ATTEMPTS,
            temp_path.display()
        ),
    ))
}

/// 生成与 block_server 对齐的临时文件路径
/// 格式: `.syncthing.{filename}.tmp`
pub(crate) fn temp_path_for(file_path: &Path) -> std::path::PathBuf {
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
    concurrency_policy: RwLock<Option<Arc<ConcurrencyPolicy>>>,
    block_source: Option<Arc<dyn BlockSource>>,
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
            block_source: None,
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
        self.block_source = source;
        self
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
            let block_source = self.block_source.clone();
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
            Self::find_local_copy_source(folder_path, file_info, db, folder_id).await?
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
        eprintln!(
            "[puller] file={} exists={} base={:?} versioner={}",
            file_info.name,
            file_path.exists(),
            local_info.as_ref().and_then(|i| i.base_version.as_ref()),
            versioner.is_some()
        );
        if file_path.exists() {
            if let Some(base) = local_info.as_ref().and_then(|i| i.base_version.as_ref()) {
                if is_mergeable_text(&file_info.name) {
                    if let (Ok(local_content), Ok(remote_content)) = (
                        fs::read_to_string(&file_path).await,
                        fs::read_to_string(&temp_path).await,
                    ) {
                        if let Some(v) = versioner {
                            eprintln!("[puller] attempting recover base");
                            if let Some(base_content) =
                                Self::recover_base_content(&**v, &file_path, base).await
                            {
                                eprintln!("[puller] base recovered, merging...");
                                let merged = three_way_merge(
                                    &base_content,
                                    &local_content,
                                    &remote_content,
                                    &file_info.name,
                                );
                                eprintln!("[puller] merged content: {}", merged.content);
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
                                    let hash = Self::content_hash(merged.content.as_bytes());
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

    /// 从 versioner 归档中恢复 base 版本内容
    ///
    /// 通过 content_hash 匹配，而不是文件修改时间，因为归档文件的 mtime
    /// 通常由 archive 时刻决定，不一定与原始 base 版本相同。
    async fn recover_base_content(
        versioner: &dyn Versioner,
        file_path: &Path,
        base: &FileInfoBase,
    ) -> Option<String> {
        let expected_hash = base.content_hash.as_ref()?;
        eprintln!("[recover] expected_hash={:?}", expected_hash);
        let versions = versioner.get_versions(file_path).await.ok()?;
        eprintln!("[recover] versions={}", versions.len());
        for version in versions {
            eprintln!(
                "[recover] version_time={:?} size={}",
                version.version_time, version.size
            );
            // 使用临时目录 restore，避免覆盖本地文件
            let tmp_dir = std::env::temp_dir().join(format!(
                "syncthing-base-{}-{:x}",
                std::process::id(),
                version
                    .version_time
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            ));
            let _ = std::fs::create_dir_all(&tmp_dir);
            let file_name = file_path.file_name().and_then(|n| n.to_str())?;
            let tmp_path = tmp_dir.join(file_name);
            eprintln!("[recover] trying restore to {:?}", tmp_path);
            match versioner.restore(&tmp_path, version.version_time).await {
                Ok(()) => {
                    if let Ok(content) = fs::read(&tmp_path).await {
                        let text = String::from_utf8_lossy(&content);
                        let hash = sha2::Sha256::digest(&content).to_vec();
                        eprintln!(
                            "[recover] restored content={:?} hash={:?} expected={:?}",
                            text, hash, expected_hash
                        );
                        if hash == *expected_hash {
                            let content = String::from_utf8(content).ok();
                            let _ = std::fs::remove_dir_all(&tmp_dir);
                            return content;
                        }
                    }
                    let _ = std::fs::remove_dir_all(&tmp_dir);
                }
                Err(e) => {
                    eprintln!("[recover] restore failed: {}", e);
                    let _ = std::fs::remove_dir_all(&tmp_dir);
                }
            }
        }
        None
    }

    /// 计算字节数组的内容哈希
    fn content_hash(data: &[u8]) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        Sha256::digest(data).to_vec()
    }

    /// 查找本地具有相同块哈希的文件（重命名优化）
    async fn find_local_copy_source(
        folder_path: &Path,
        file_info: &FileInfo,
        db: &dyn LocalDatabase,
        folder_id: &str,
    ) -> Result<Option<std::path::PathBuf>> {
        if file_info.blocks.is_empty() || file_info.is_deleted() {
            return Ok(None);
        }

        let db_files = db.get_folder_files(folder_id).await?;
        for db_file in db_files {
            if db_file.is_deleted() || db_file.name == file_info.name {
                continue;
            }
            if db_file.blocks.len() != file_info.blocks.len() {
                continue;
            }
            let same_blocks = db_file
                .blocks
                .iter()
                .zip(file_info.blocks.iter())
                .all(|(a, b)| a.hash == b.hash);
            if same_blocks {
                let source_path = folder_path.join(&db_file.name);
                if source_path.exists() && source_path.is_file() {
                    return Ok(Some(source_path));
                }
            }
        }
        Ok(None)
    }

    /// 创建目录
    async fn create_directory(folder_path: &Path, file_info: &FileInfo) -> Result<()> {
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
