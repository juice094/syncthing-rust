//! 文件夹模型实现
//!
//! 实现文件夹级别的扫描和拉取循环

use crate::database::LocalDatabase;
use crate::error::Result;
use crate::events::{EventPublisher, SyncEvent};
use crate::model::FolderState;
use crate::orchestrator::FolderOrchestrator;
use crate::puller::{BlockSource, ConcurrencyPolicy, Puller};
use crate::scanner::Scanner;
use crate::watcher::FolderWatcher;
use dashmap::DashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock as SyncRwLock};
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
    concurrency_policy: SyncRwLock<Option<Arc<ConcurrencyPolicy>>>,
    orchestrator: SyncRwLock<Option<Arc<FolderOrchestrator>>>,
    watcher: RwLock<Option<notify::RecommendedWatcher>>,
    pull_notify: tokio::sync::Notify,
    pending_pulls: RwLock<Vec<FileInfo>>,
    /// watcher 观测到的变更路径集合（用于增量扫描）
    dirty_changes: Arc<DashSet<String>>,
    /// watcher 观测到的删除路径集合
    dirty_deletes: Arc<DashSet<String>>,
    /// watcher 收到的事件总数
    watcher_events_received: Arc<AtomicU64>,
    /// watcher 通道丢弃的事件数（由 watcher.rs 维护）
    watcher_dropped: Arc<AtomicU64>,
    /// 上一次已报告的丢弃数，用于计算差值
    last_reported_dropped: AtomicU64,
}

impl FolderModel {
    /// 创建新的文件夹模型
    pub fn new(
        folder: Folder,
        db: Arc<dyn LocalDatabase>,
        events: EventPublisher,
        block_source: Option<Arc<dyn BlockSource>>,
    ) -> Self {
        Self::new_with_policy_and_orchestrator(folder, db, events, block_source, None, None)
    }

    /// 创建文件夹模型并指定共享并发策略与编排器
    fn new_with_policy_and_orchestrator(
        folder: Folder,
        db: Arc<dyn LocalDatabase>,
        events: EventPublisher,
        block_source: Option<Arc<dyn BlockSource>>,
        concurrency_policy: Option<Arc<ConcurrencyPolicy>>,
        orchestrator: Option<Arc<FolderOrchestrator>>,
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
            .with_versioner(versioner)
            .with_concurrency_policy(concurrency_policy.clone());
        let folder_id = folder.id.clone();
        Self {
            folder,
            state: RwLock::new(FolderState::new(folder_id)),
            db,
            events,
            scanner,
            puller,
            concurrency_policy: SyncRwLock::new(concurrency_policy),
            orchestrator: SyncRwLock::new(orchestrator),
            watcher: RwLock::new(None),
            pull_notify: tokio::sync::Notify::new(),
            pending_pulls: RwLock::new(Vec::new()),
            dirty_changes: Arc::new(DashSet::new()),
            dirty_deletes: Arc::new(DashSet::new()),
            watcher_events_received: Arc::new(AtomicU64::new(0)),
            watcher_dropped: Arc::new(AtomicU64::new(0)),
            last_reported_dropped: AtomicU64::new(0),
        }
    }

    /// 设置共享并发策略（构建器模式）
    pub fn with_concurrency_policy(mut self, policy: Option<Arc<ConcurrencyPolicy>>) -> Self {
        self.concurrency_policy = SyncRwLock::new(policy.clone());
        self.puller = self.puller.with_concurrency_policy(policy);
        self
    }

    /// 动态更新共享并发策略
    pub fn set_concurrency_policy(&self, policy: Arc<ConcurrencyPolicy>) {
        if let Ok(mut guard) = self.concurrency_policy.write() {
            *guard = Some(policy.clone());
        }
        self.puller.set_concurrency_policy(Some(policy));
    }

    /// 设置文件夹编排器（构建器模式）
    pub fn with_orchestrator(mut self, orchestrator: Option<Arc<FolderOrchestrator>>) -> Self {
        self.orchestrator = SyncRwLock::new(orchestrator);
        self
    }

    /// 动态更新文件夹编排器
    pub fn set_orchestrator(&self, orchestrator: Arc<FolderOrchestrator>) {
        if let Ok(mut guard) = self.orchestrator.write() {
            *guard = Some(orchestrator);
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
                    let orchestrator = self.orchestrator.read().ok().and_then(|g| g.clone());
                    let _permit = if let Some(ref orch) = orchestrator {
                        Some(orch.clone().begin_scan(&self.folder.id).await)
                    } else {
                        None
                    };
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

        let mut rx = match FolderWatcher::watch(&folder_id, &path, self.watcher_dropped.clone()) {
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

        // 绑定共享集合，供同步 watcher 回调使用
        let dirty_changes = self.dirty_changes.clone();
        let dirty_deletes = self.dirty_deletes.clone();
        let events_received = self.watcher_events_received.clone();

        loop {
            tokio::select! {
                Some(event) = rx.recv() => {
                    events_received.fetch_add(1, Ordering::Relaxed);

                    let is_remove = matches!(event.kind, notify::EventKind::Remove(_));

                    for p in event.paths {
                        let relative = match Self::relative_path(&path, &p) {
                            Some(r) => r,
                            None => continue,
                        };

                        // Skip events for syncthing temp files to break positive feedback
                        if relative.contains(".syncthing.") && relative.ends_with(".tmp") {
                            trace!(folder_id = %folder_id, path = %relative, "Skipping watcher event for syncthing temp");
                            continue;
                        }

                        trace!(folder_id = %folder_id, path = %relative, is_remove, "Watcher event recorded");

                        if is_remove {
                            dirty_deletes.insert(relative);
                        } else {
                            dirty_changes.insert(relative);
                        }
                    }

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
                    let orchestrator = self.orchestrator.read().ok().and_then(|g| g.clone());
                    let _permit = if let Some(ref orch) = orchestrator {
                        Some(orch.clone().begin_scan(&folder_id).await)
                    } else {
                        None
                    };
                    if let Err(e) = self.process_dirty_set().await {
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

    /// 将绝对路径转换为 folder 根目录下的相对路径（统一正斜杠）。
    fn relative_path(folder_path: &str, abs_path: &Path) -> Option<String> {
        let base = Path::new(folder_path);
        abs_path
            .strip_prefix(base)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    /// 启动拉取循环
    pub async fn start_pull_loop(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        info!(folder_id = %self.folder.id, "Starting pull loop");

        let mut interval = tokio::time::interval(Duration::from_secs(10));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let orchestrator = self.orchestrator.read().ok().and_then(|g| g.clone());
                    let _permit = if let Some(ref orch) = orchestrator {
                        Some(orch.clone().begin_pull(&self.folder.id).await)
                    } else {
                        None
                    };
                    if let Err(e) = self.pull().await {
                        error!(folder_id = %self.folder.id, error = %e, "Pull failed");
                    }
                }
                _ = self.pull_notify.notified() => {
                    trace!(folder_id = %self.folder.id, "Pull triggered by remote index");
                    let orchestrator = self.orchestrator.read().ok().and_then(|g| g.clone());
                    let _permit = if let Some(ref orch) = orchestrator {
                        Some(orch.clone().begin_pull(&self.folder.id).await)
                    } else {
                        None
                    };
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

    /// 处理 watcher 累积的脏路径集合，决定全量或增量扫描。
    async fn process_dirty_set(&self) -> Result<Vec<FileInfo>> {
        let folder_id = self.folder.id.clone();

        // 取出并清空脏集合
        let changes: Vec<String> = {
            let mut v = Vec::with_capacity(self.dirty_changes.len());
            for entry in self.dirty_changes.iter() {
                v.push(entry.clone());
            }
            self.dirty_changes.clear();
            v
        };
        let deletes: Vec<String> = {
            let mut v = Vec::with_capacity(self.dirty_deletes.len());
            for entry in self.dirty_deletes.iter() {
                v.push(entry.clone());
            }
            self.dirty_deletes.clear();
            v
        };

        let dirty_count = changes.len() + deletes.len();
        if dirty_count == 0 {
            return Ok(vec![]);
        }

        self.events.publish(SyncEvent::WatcherDirtySetSize {
            folder: folder_id.clone(),
            size: dirty_count,
        });

        // 报告 watcher 丢事件数
        let total_dropped = self.watcher_dropped.load(Ordering::Relaxed);
        let last = self.last_reported_dropped.load(Ordering::Relaxed);
        if total_dropped > last {
            self.last_reported_dropped
                .store(total_dropped, Ordering::Relaxed);
            self.events.publish(SyncEvent::WatcherEventsDropped {
                folder: folder_id.clone(),
                dropped: total_dropped - last,
            });
        }

        // 阈值：取 max(100, local_files / 10)，避免小 folder 过度碎片化
        let local_files = self.state.read().await.local_files;
        let threshold = std::cmp::max(100usize, local_files / 10);

        // 如果脏路径包含根目录或超过阈值，回退到全量扫描
        let fallback_to_full = dirty_count > threshold
            || changes.iter().any(|p| p.is_empty() || p == "/")
            || deletes.iter().any(|p| p.is_empty() || p == "/");

        if fallback_to_full {
            info!(
                folder_id = %folder_id,
                dirty_count = dirty_count,
                threshold = threshold,
                "Dirty set too large, falling back to full scan"
            );
            return self.scan().await;
        }

        info!(
            folder_id = %folder_id,
            changes = changes.len(),
            deletes = deletes.len(),
            "Incremental scan triggered by watcher"
        );

        self.events.publish(SyncEvent::IncrementalScanTriggered {
            folder: folder_id.clone(),
            paths: dirty_count,
        });

        self.scan_incremental(changes, deletes).await
    }

    /// 执行增量扫描：只扫描脏路径涉及的子树，并处理显式删除事件。
    async fn scan_incremental(
        &self,
        changes: Vec<String>,
        deletes: Vec<String>,
    ) -> Result<Vec<FileInfo>> {
        let folder_id = self.folder.id.clone();

        // 进入 Scanning 状态
        let old_status = {
            let mut state = self.state.write().await;
            let old = state.status;
            state.status = FolderStatus::Scanning;
            old
        };
        self.events.publish(SyncEvent::FolderStateChanged {
            folder: folder_id.clone(),
            from: old_status,
            to: FolderStatus::Scanning,
        });

        let mut changed_files = Vec::new();

        // 1. 处理变更路径：目录用 scan_sub，单文件用 scan_changed_file
        let base_path = Path::new(&self.folder.path);
        for relative in changes {
            let full = base_path.join(&relative);
            match tokio::fs::metadata(&full).await {
                Ok(m) if m.is_dir() => {
                    match self.scanner.scan_folder_sub(&self.folder, &relative).await {
                        Ok(mut files) => changed_files.append(&mut files),
                        Err(e) => {
                            error!(folder_id = %folder_id, root = %relative, error = %e, "Incremental scan subtree failed");
                        }
                    }
                    // 补全子树内删除的文件
                    match self
                        .scanner
                        .mark_deleted_subtree(&self.folder, &relative)
                        .await
                    {
                        Ok(mut files) => changed_files.append(&mut files),
                        Err(e) => {
                            error!(folder_id = %folder_id, root = %relative, error = %e, "Incremental scan subtree delete check failed");
                        }
                    }
                }
                Ok(_) => {
                    match self
                        .scanner
                        .scan_changed_file(&self.folder, &relative)
                        .await
                    {
                        Ok(Some(file)) => changed_files.push(file),
                        Ok(None) => {}
                        Err(e) => {
                            error!(folder_id = %folder_id, path = %relative, error = %e, "Incremental scan file failed");
                        }
                    }
                }
                Err(e) => {
                    trace!(folder_id = %folder_id, path = %relative, error = %e, "Changed path no longer exists, skipping");
                }
            }
        }

        // 2. 处理显式删除事件（文件或子树）
        for relative in deletes {
            match self
                .scanner
                .mark_deleted_subtree(&self.folder, &relative)
                .await
            {
                Ok(mut files) => changed_files.append(&mut files),
                Err(e) => {
                    error!(folder_id = %folder_id, path = %relative, error = %e, "Mark deleted subtree failed");
                }
            }
        }

        // 3. 重命名检测与 blocks 清理
        changed_files = Scanner::detect_and_reorder_renames(changed_files);
        for file in &mut changed_files {
            if file.is_deleted() {
                file.blocks.clear();
            }
        }

        // 4. 更新数据库
        if let Err(e) = self
            .db
            .update_files(&folder_id, changed_files.clone())
            .await
        {
            error!(folder_id = %folder_id, error = %e, "Incremental scan DB update failed");

            let mut state = self.state.write().await;
            state.status = old_status;
            state.errors.push(e.to_string());
            self.events.publish(SyncEvent::FolderStateChanged {
                folder: folder_id.clone(),
                from: FolderStatus::Scanning,
                to: old_status,
            });
            return Err(e);
        }

        // 5. 发布索引更新
        if !changed_files.is_empty() {
            self.events.publish(SyncEvent::LocalIndexUpdated {
                folder: folder_id.clone(),
                files: changed_files.clone(),
            });
        }

        // 6. 更新状态
        {
            let mut state = self.state.write().await;
            state.status = FolderStatus::Idle;
            state.last_scan = Some(chrono::Utc::now());
            if let Ok(all_files) = self.db.get_folder_files(&folder_id).await {
                state.local_files = all_files.len();
            }
        }
        self.events.publish(SyncEvent::FolderStateChanged {
            folder: folder_id.clone(),
            from: FolderStatus::Scanning,
            to: FolderStatus::Idle,
        });
        self.events.publish(SyncEvent::FolderScanCompleted {
            folder: folder_id,
            files_changed: changed_files.len(),
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
