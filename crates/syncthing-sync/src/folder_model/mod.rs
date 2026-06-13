//! 文件夹模型实现
//!
//! 实现文件夹级别的扫描和拉取循环

use crate::database::LocalDatabase;
use crate::error::Result;
use crate::events::{EventPublisher, SyncEvent};
use crate::model::FolderState;
use crate::puller::{BlockSource, Puller};
use crate::scanner::Scanner;
use crate::watcher::FolderWatcher;
use std::sync::Arc;
use std::time::Duration;
use syncthing_core::types::{FileInfo, Folder, FolderStatus, FolderType};
use syncthing_core::DeviceId;
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace, warn};

/// 文件夹模型
pub struct FolderModel {
    folder: Folder,
    state: RwLock<FolderState>,
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    scanner: Scanner,
    puller: Puller,
    watcher: RwLock<Option<notify::RecommendedWatcher>>,
    pull_notify: tokio::sync::Notify,
    pending_pulls: RwLock<Vec<FileInfo>>,
}

impl FolderModel {
    /// 创建新的文件夹模型
    pub fn new(
        folder: Folder,
        db: Arc<dyn LocalDatabase>,
        events: EventPublisher,
        block_source: Option<Arc<dyn BlockSource>>,
    ) -> Self {
        let scanner = Scanner::new(db.clone(), events.clone());
        let versioner: Option<Arc<dyn syncthing_versioner::Versioner>> = folder
            .versioning
            .as_ref()
            .and_then(|cfg| {
                syncthing_versioner::create_versioner(cfg, std::path::Path::new(&folder.path))
            })
            .map(Arc::from);
        let puller = Puller::new(db.clone(), events.clone())
            .with_block_source(block_source)
            .with_versioner(versioner);
        let folder_id = folder.id.clone();
        Self {
            folder,
            state: RwLock::new(FolderState::new(folder_id)),
            db,
            events,
            scanner,
            puller,
            watcher: RwLock::new(None),
            pull_notify: tokio::sync::Notify::new(),
            pending_pulls: RwLock::new(Vec::new()),
        }
    }

    /// 获取文件夹ID
    pub fn id(&self) -> &str {
        &self.folder.id
    }

    /// 获取文件夹配置
    pub fn config(&self) -> &Folder {
        &self.folder
    }

    /// 获取文件夹状态
    pub async fn state(&self) -> FolderState {
        self.state.read().await.clone()
    }

    /// 启动扫描循环
    pub async fn start_scan_loop(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let interval_secs = if self.folder.rescan_interval_secs > 0 {
            self.folder.rescan_interval_secs as u64
        } else {
            syncthing_core::constants::DEFAULT_SCAN_INTERVAL_SECS
        };

        info!(
            folder_id = %self.folder.id,
            interval_secs = interval_secs,
            "Starting scan loop"
        );

        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    debug!(folder_id = %self.folder.id, "Scan loop tick triggered by interval");
                    if let Err(e) = self.scan().await {
                        error!(folder_id = %self.folder.id, error = %e, "Scan failed");
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!(folder_id = %self.folder.id, "Scan loop shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// 启动文件系统监视循环
    pub async fn start_watcher_loop(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let folder_id = self.folder.id.clone();
        let path = self.folder.path.clone();

        let mut rx = match FolderWatcher::watch(&folder_id, &path) {
            Ok((w, rx)) => {
                *self.watcher.write().await = Some(w);
                rx
            }
            Err(e) => {
                error!(folder_id = %folder_id, error = %e, "Failed to start folder watcher");
                return;
            }
        };

        const WATCHER_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(5);
        const MIN_SCAN_GAP: std::time::Duration = std::time::Duration::from_secs(5);

        let mut debounce_timer: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
        let mut last_scan = tokio::time::Instant::now() - MIN_SCAN_GAP;

        loop {
            tokio::select! {
                Some(event) = rx.recv() => {
                    // Skip events for syncthing temp files to break positive feedback
                    let skip = event.paths.iter().any(|p| {
                        let s = p.to_string_lossy();
                        s.contains(".syncthing.") && s.ends_with(".tmp")
                    });
                    if skip {
                        trace!(folder_id = %folder_id, "Skipping watcher event for syncthing temp");
                        continue;
                    }
                    trace!(folder_id = %folder_id, event = ?event, "Watcher event received");
                    debounce_timer = Some(Box::pin(tokio::time::sleep(WATCHER_DEBOUNCE)));
                }
                _ = async {
                    if let Some(ref mut timer) = debounce_timer {
                        timer.as_mut().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                }, if debounce_timer.is_some() => {
                    debounce_timer = None;
                    let elapsed = last_scan.elapsed();
                    if elapsed < MIN_SCAN_GAP {
                        trace!(folder_id = %folder_id, elapsed_s = elapsed.as_secs(), "Scan skipped: min gap");
                        continue;
                    }
                    info!(folder_id = %folder_id, "Debounced watcher scan triggered");
                    if let Err(e) = self.scan().await {
                        error!(folder_id = %folder_id, error = %e, "Watcher-triggered scan failed");
                    }
                    last_scan = tokio::time::Instant::now();
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!(folder_id = %folder_id, "Folder watcher loop shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// 启动拉取循环
    pub async fn start_pull_loop(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        info!(folder_id = %self.folder.id, "Starting pull loop");

        let mut interval = tokio::time::interval(Duration::from_secs(10));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.pull().await {
                        error!(folder_id = %self.folder.id, error = %e, "Pull failed");
                    }
                }
                _ = self.pull_notify.notified() => {
                    trace!(folder_id = %self.folder.id, "Pull triggered by remote index");
                    if let Err(e) = self.pull().await {
                        error!(folder_id = %self.folder.id, error = %e, "Pull failed");
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!(folder_id = %self.folder.id, "Pull loop shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// 执行扫描
    pub async fn scan(&self) -> Result<Vec<FileInfo>> {
        self.scan_inner(None).await
    }

    /// 扫描子目录
    pub async fn scan_sub(&self, sub: &str) -> Result<Vec<FileInfo>> {
        self.scan_inner(Some(sub)).await
    }

    async fn scan_inner(&self, sub: Option<&str>) -> Result<Vec<FileInfo>> {
        if self.folder.paused {
            debug!(folder_id = %self.folder.id, "Folder is paused, skipping scan");
            return Ok(vec![]);
        }

        let mut state = self.state.write().await;

        // 更新状态
        let old_status = state.status;
        state.status = FolderStatus::Scanning;
        drop(state);

        self.events.publish(SyncEvent::FolderStateChanged {
            folder: self.folder.id.clone(),
            from: old_status,
            to: FolderStatus::Scanning,
        });

        info!(folder_id = %self.folder.id, sub = ?sub, "Starting scan");

        // 执行扫描
        let scan_result = match sub {
            Some(s) => self.scanner.scan_folder_sub(&self.folder, s).await,
            None => self.scanner.scan_folder(&self.folder).await,
        };

        let changed_files = match scan_result {
            Ok(files) => {
                let changed_count = files.len();
                info!(
                    folder_id = %self.folder.id,
                    changed_count = changed_count,
                    "Scan completed"
                );

                // 如果有变更，发布索引更新事件
                if !files.is_empty() {
                    self.events.publish(SyncEvent::LocalIndexUpdated {
                        folder: self.folder.id.clone(),
                        files: files.clone(),
                    });
                }

                files
            }
            Err(e) => {
                let err_str = e.to_string();
                error!(folder_id = %self.folder.id, error = %err_str, "Scan failed");

                let mut state = self.state.write().await;
                state.errors.push(err_str);

                // 恢复状态
                state.status = old_status;
                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Scanning,
                    to: old_status,
                });

                return Err(e);
            }
        };

        // 更新状态
        let mut state = self.state.write().await;
        state.status = FolderStatus::Idle;
        state.last_scan = Some(chrono::Utc::now());

        // 更新文件计数
        if let Ok(all_files) = self.db.get_folder_files(&self.folder.id).await {
            let files: &Vec<syncthing_core::types::FileInfo> = &all_files;
            state.local_files = files.len();
        }

        self.events.publish(SyncEvent::FolderStateChanged {
            folder: self.folder.id.clone(),
            from: FolderStatus::Scanning,
            to: FolderStatus::Idle,
        });

        Ok(changed_files)
    }

    /// 执行拉取
    pub async fn pull(&self) -> Result<()> {
        if self.folder.paused {
            debug!(folder_id = %self.folder.id, "Folder is paused, skipping pull");
            return Ok(());
        }

        if !self.folder.folder_type.can_sync() {
            debug!(folder_id = %self.folder.id, "Folder type cannot sync, skipping pull");
            return Ok(());
        }

        let mut state = self.state.write().await;

        // 如果已经在拉取中，跳过
        if state.status == FolderStatus::Pulling {
            trace!(folder_id = %self.folder.id, "Already pulling, skipping");
            return Ok(());
        }

        let old_status = state.status;
        state.status = FolderStatus::Pulling;
        drop(state);

        self.events.publish(SyncEvent::FolderStateChanged {
            folder: self.folder.id.clone(),
            from: old_status,
            to: FolderStatus::Pulling,
        });

        debug!(folder_id = %self.folder.id, "Starting pull");

        // 获取需要拉取的文件列表
        // 1. 先检查 pending_pulls（由远程索引触发）
        let mut pending_files = {
            let mut pending = self.pending_pulls.write().await;
            std::mem::take(&mut *pending)
        };

        // 2. 再检查文件系统状态（补充本地缺失的文件）
        let fs_needed = match self.puller.check_needed_files(&self.folder).await {
            Ok(files) => files,
            Err(e) => {
                error!(folder_id = %self.folder.id, error = %e, "Failed to check needed files");

                let mut state = self.state.write().await;
                state.status = old_status;
                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Pulling,
                    to: old_status,
                });

                return Err(e);
            }
        };

        // 合并 pending_pulls 和 fs_needed，去重
        for file in fs_needed {
            if !pending_files.iter().any(|f| f.name == file.name) {
                pending_files.push(file);
            }
        }
        let needed_files = pending_files;

        if needed_files.is_empty() {
            debug!(folder_id = %self.folder.id, "No files need pulling");

            let mut state = self.state.write().await;
            state.status = old_status;
            self.events.publish(SyncEvent::FolderStateChanged {
                folder: self.folder.id.clone(),
                from: FolderStatus::Pulling,
                to: old_status,
            });

            return Ok(());
        }

        info!(
            folder_id = %self.folder.id,
            file_count = needed_files.len(),
            "Pulling files"
        );

        // 更新状态
        {
            let mut state = self.state.write().await;
            state.need_files = needed_files.len();
        }

        // 执行拉取
        match self.puller.pull_folder(&self.folder, needed_files).await {
            Ok(stats) => {
                info!(
                    folder_id = %self.folder.id,
                    succeeded = stats.files_succeeded,
                    failed = stats.files_failed,
                    "Pull completed"
                );

                // 更新状态
                let mut state = self.state.write().await;
                state.status = FolderStatus::Idle;
                state.last_pull = Some(chrono::Utc::now());
                state.need_files = 0;

                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Pulling,
                    to: FolderStatus::Idle,
                });

                self.events.publish(SyncEvent::SyncComplete {
                    folder: self.folder.id.clone(),
                    stats: crate::events::SyncStats {
                        files_added: stats.files_succeeded,
                        ..Default::default()
                    },
                });
            }
            Err(e) => {
                error!(folder_id = %self.folder.id, error = %e, "Pull failed");

                let mut state = self.state.write().await;
                state.status = old_status;
                state.errors.push(e.to_string());

                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Pulling,
                    to: old_status,
                });

                return Err(e);
            }
        }

        Ok(())
    }

    /// 处理远程索引
    pub async fn handle_remote_index(&self, device: DeviceId, files: Vec<FileInfo>) -> Result<()> {
        debug!(
            folder_id = %self.folder.id,
            device = %device.short_id(),
            file_count = files.len(),
            "Handling remote index"
        );

        // 将需要拉取的文件加入 pending_pulls
        if !files.is_empty() {
            let mut pending = self.pending_pulls.write().await;
            for file in files {
                if !pending.iter().any(|f| f.name == file.name) {
                    pending.push(file);
                }
            }
            drop(pending);
        }

        // 唤醒 pull loop 立即处理远程索引
        // 使用 notify_one 而非 notify_waiters，确保即使 pull loop 正在执行 pull()
        // 也能在完成后立即收到通知，不会丢失唤醒信号。
        self.pull_notify.notify_one();

        Ok(())
    }

    /// 更新文件夹配置
    pub async fn update_config(&mut self, config: Folder) {
        self.folder = config;
        self.events.publish(SyncEvent::FolderConfigUpdated {
            folder: self.folder.clone(),
        });
    }

    /// 暂停文件夹
    pub async fn pause(&self) {
        let mut state = self.state.write().await;
        state.status = FolderStatus::Idle;
        info!(folder_id = %self.folder.id, "Folder paused");
    }

    /// 恢复文件夹
    pub async fn resume(&self) {
        info!(folder_id = %self.folder.id, "Folder resumed");
    }

    /// Override local changes for a ReceiveOnly folder.
    /// Scans local files, increments version vectors, and updates the database
    /// so local changes are treated as authoritative and broadcast to peers.
    pub async fn override_local_changes(&self) -> Result<()> {
        if !matches!(self.folder.folder_type, FolderType::ReceiveOnly) {
            return Err(crate::SyncError::scan(
                self.folder.id.clone(),
                "override only applies to ReceiveOnly folders".to_string(),
            ));
        }

        let changed = self.scan().await?;
        if changed.is_empty() {
            info!(folder_id = %self.folder.id, "No local changes to override");
            return Ok(());
        }

        let count = changed.len();
        let mut updated = Vec::with_capacity(count);
        for mut file in changed {
            file.version.increment(1);
            updated.push(file);
        }

        self.db
            .update_files(&self.folder.id, updated)
            .await
            .map_err(|e| {
                crate::SyncError::scan(self.folder.id.clone(), format!("db update failed: {}", e))
            })?;

        info!(folder_id = %self.folder.id, count = count, "Override accepted local changes");
        Ok(())
    }

    /// Revert local changes for a ReceiveOnly folder.
    /// Deletes locally modified or added files and re-triggers pull from peers.
    pub async fn revert_local_changes(&self) -> Result<()> {
        if !matches!(self.folder.folder_type, FolderType::ReceiveOnly) {
            return Err(crate::SyncError::scan(
                self.folder.id.clone(),
                "revert only applies to ReceiveOnly folders".to_string(),
            ));
        }

        let base_path = std::path::Path::new(&self.folder.path);
        let remote_files = self.db.get_folder_files(&self.folder.id).await?;

        let mut deleted_count = 0;
        for remote in &remote_files {
            let local_path = base_path.join(&remote.name);
            if !local_path.exists() {
                continue;
            }

            let should_delete = if remote.is_deleted() {
                true
            } else {
                match tokio::fs::metadata(&local_path).await {
                    Ok(meta) => meta.len() != remote.size as u64,
                    Err(_) => true,
                }
            };

            if should_delete {
                if let Err(e) = tokio::fs::remove_file(&local_path).await {
                    warn!(
                        "Failed to delete {} for revert: {}",
                        local_path.display(),
                        e
                    );
                } else {
                    deleted_count += 1;
                }
                if let Err(e) = self.db.delete_file(&self.folder.id, &remote.name).await {
                    warn!("Failed to delete db record {}: {}", remote.name, e);
                }
            }
        }

        self.scan().await?;
        self.pull().await?;

        info!(folder_id = %self.folder.id, deleted = deleted_count, "Revert completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests;
