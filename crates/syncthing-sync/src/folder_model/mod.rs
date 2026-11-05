//! Folder model.
//!
//! Implements the per-folder scan and pull loops.

use crate::database::LocalDatabase;
use crate::error::Result;
use crate::events::{EventPublisher, SyncEvent};
use crate::model::FolderState;
use crate::orchestrator::FolderOrchestrator;
use crate::puller::{BlockSource, ConcurrencyPolicy, PullStats, Puller};
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

/// Per-folder runtime: scan/pull/watcher loops and state machine.
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
    /// Paths of changes observed by the watcher (for incremental scans).
    dirty_changes: Arc<DashSet<String>>,
    /// Paths of deletions observed by the watcher.
    dirty_deletes: Arc<DashSet<String>>,
    /// Total number of events received by the watcher.
    watcher_events_received: Arc<AtomicU64>,
    /// Number of events dropped by the watcher channel (maintained by watcher.rs).
    watcher_dropped: Arc<AtomicU64>,
    /// Last reported drop count, used to compute deltas.
    last_reported_dropped: AtomicU64,
    /// Local version-vector counter ID (used by local increment paths such as override).
    local_counter_id: u64,
}

impl FolderModel {
    /// Creates a new folder model.
    pub fn new(
        folder: Folder,
        db: Arc<dyn LocalDatabase>,
        events: EventPublisher,
        block_source: Option<Arc<dyn BlockSource>>,
        local_counter_id: u64,
    ) -> Self {
        Self::new_with_policy_and_orchestrator(
            folder,
            db,
            events,
            block_source,
            None,
            None,
            local_counter_id,
        )
    }

    /// Creates a folder model with the shared concurrency policy and orchestrator.
    fn new_with_policy_and_orchestrator(
        folder: Folder,
        db: Arc<dyn LocalDatabase>,
        events: EventPublisher,
        block_source: Option<Arc<dyn BlockSource>>,
        concurrency_policy: Option<Arc<ConcurrencyPolicy>>,
        orchestrator: Option<Arc<FolderOrchestrator>>,
        local_counter_id: u64,
    ) -> Self {
        let scanner = Scanner::new(db.clone(), events.clone(), local_counter_id);
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
            local_counter_id,
        }
    }

    /// Sets the shared concurrency policy (builder style).
    pub fn with_concurrency_policy(mut self, policy: Option<Arc<ConcurrencyPolicy>>) -> Self {
        self.concurrency_policy = SyncRwLock::new(policy.clone());
        self.puller = self.puller.with_concurrency_policy(policy);
        self
    }

    /// Updates the shared concurrency policy at runtime.
    pub fn set_concurrency_policy(&self, policy: Arc<ConcurrencyPolicy>) {
        if let Ok(mut guard) = self.concurrency_policy.write() {
            *guard = Some(policy.clone());
        }
        self.puller.set_concurrency_policy(Some(policy));
    }

    /// Updates the block source at runtime (allows wiring it in after creation).
    pub fn set_block_source(&self, source: Option<Arc<dyn BlockSource>>) {
        self.puller.set_block_source(source);
    }

    /// 测试辅助：puller 是否已配置块数据源
    #[cfg(test)]
    pub(crate) fn puller_block_source_present(&self) -> bool {
        self.puller.block_source_present()
    }

    /// Sets the folder orchestrator (builder style).
    pub fn with_orchestrator(mut self, orchestrator: Option<Arc<FolderOrchestrator>>) -> Self {
        self.orchestrator = SyncRwLock::new(orchestrator);
        self
    }

    /// Updates the folder orchestrator at runtime.
    pub fn set_orchestrator(&self, orchestrator: Arc<FolderOrchestrator>) {
        if let Ok(mut guard) = self.orchestrator.write() {
            *guard = Some(orchestrator);
        }
    }

    /// Returns the folder ID.
    pub fn id(&self) -> &str {
        &self.folder.id
    }

    /// Returns the folder configuration.
    pub fn config(&self) -> &Folder {
        &self.folder
    }

    /// Returns a snapshot of the folder state.
    pub async fn state(&self) -> FolderState {
        self.state.read().await.clone()
    }

    /// Runs the periodic scan loop until the shutdown signal fires.
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

    /// Runs the filesystem watcher loop (debounced incremental scans) until
    /// the shutdown signal fires.
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

    /// Converts an absolute path to a path relative to the folder root
    /// (normalized to forward slashes).
    fn relative_path(folder_path: &str, abs_path: &Path) -> Option<String> {
        let base = Path::new(folder_path);
        abs_path
            .strip_prefix(base)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    /// Runs the pull loop until the shutdown signal fires.
    ///
    /// After a round with file-level failures, the next round is scheduled
    /// earlier using exponential backoff (`PULL_RETRY_BACKOFF_INIT_MS`,
    /// doubling up to `PULL_RETRY_BACKOFF_MAX_MS`). Failed files are not
    /// tracked here: `check_needed_files()` re-derives them from the local
    /// DB/FS diff, so the backoff only affects retry timing (see
    /// `docs/KNOWN_ISSUES.md` §23).
    pub async fn start_pull_loop(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        info!(folder_id = %self.folder.id, "Starting pull loop");

        let mut interval = tokio::time::interval(Duration::from_secs(
            syncthing_core::constants::PULL_INTERVAL_SECS,
        ));
        let mut backoff = Duration::ZERO;

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = self.pull_notify.notified() => {
                    trace!(folder_id = %self.folder.id, "Pull triggered by remote index");
                }
                _ = tokio::time::sleep(backoff), if !backoff.is_zero() => {
                    debug!(
                        folder_id = %self.folder.id,
                        backoff_ms = backoff.as_millis() as u64,
                        "Retrying pull after previous failures"
                    );
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!(folder_id = %self.folder.id, "Pull loop shutting down");
                        break;
                    }
                }
            }

            let orchestrator = self.orchestrator.read().ok().and_then(|g| g.clone());
            let _permit = if let Some(ref orch) = orchestrator {
                Some(orch.clone().begin_pull(&self.folder.id).await)
            } else {
                None
            };

            backoff = match self.pull().await {
                Ok(stats) if stats.files_failed > 0 => {
                    let next = Self::next_pull_backoff(backoff);
                    warn!(
                        folder_id = %self.folder.id,
                        failed = stats.files_failed,
                        next_retry_ms = next.as_millis() as u64,
                        "Pull had failures; scheduling backoff retry"
                    );
                    next
                }
                Ok(_) => Duration::ZERO,
                Err(e) => {
                    error!(folder_id = %self.folder.id, error = %e, "Pull failed");
                    Self::next_pull_backoff(backoff)
                }
            };
        }
    }

    /// Computes the next failure-retry backoff: first
    /// `PULL_RETRY_BACKOFF_INIT_MS`, then doubling, capped at
    /// `PULL_RETRY_BACKOFF_MAX_MS`.
    fn next_pull_backoff(current: Duration) -> Duration {
        let max = Duration::from_millis(syncthing_core::constants::PULL_RETRY_BACKOFF_MAX_MS);
        if current.is_zero() {
            Duration::from_millis(syncthing_core::constants::PULL_RETRY_BACKOFF_INIT_MS)
        } else {
            (current * 2).min(max)
        }
    }

    /// Runs a full scan of the folder.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`](crate::SyncError) when the scan fails; the folder state is restored.
    pub async fn scan(&self) -> Result<Vec<FileInfo>> {
        self.scan_inner(None).await
    }

    /// Scans a subdirectory of the folder.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`](crate::SyncError) when the scan fails; the folder state is restored.
    pub async fn scan_sub(&self, sub: &str) -> Result<Vec<FileInfo>> {
        self.scan_inner(Some(sub)).await
    }

    async fn scan_inner(&self, sub: Option<&str>) -> Result<Vec<FileInfo>> {
        if self.folder.paused {
            debug!(folder_id = %self.folder.id, "Folder is paused, skipping scan");
            return Ok(vec![]);
        }

        let mut state = self.state.write().await;

        let old_status = state.status;
        state.status = FolderStatus::Scanning;
        drop(state);

        self.events.publish(SyncEvent::FolderStateChanged {
            folder: self.folder.id.clone(),
            from: old_status,
            to: FolderStatus::Scanning,
        });

        info!(folder_id = %self.folder.id, sub = ?sub, "Starting scan");

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

                if !files.is_empty() {
                    info!(
                        folder_id = %self.folder.id,
                        files = files.len(),
                        "Publishing LocalIndexUpdated event"
                    );
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

                state.status = old_status;
                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Scanning,
                    to: old_status,
                });

                return Err(e);
            }
        };

        let mut state = self.state.write().await;
        state.status = FolderStatus::Idle;
        state.last_scan = Some(chrono::Utc::now());

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

    /// Processes the dirty path sets accumulated by the watcher, choosing
    /// between a full scan and an incremental scan.
    async fn process_dirty_set(&self) -> Result<Vec<FileInfo>> {
        let folder_id = self.folder.id.clone();

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

        // Threshold: max(100, local_files / 10), so small folders do not
        // over-fragment into incremental scans.
        let local_files = self.state.read().await.local_files;
        let threshold = std::cmp::max(100usize, local_files / 10);

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

    /// Runs an incremental scan over the dirty subtrees and explicit deletions.
    ///
    /// See "docs/design/ADR-20261105-scanner-incremental-paths.md".
    async fn scan_incremental(
        &self,
        changes: Vec<String>,
        deletes: Vec<String>,
    ) -> Result<Vec<FileInfo>> {
        let folder_id = self.folder.id.clone();

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

        // Pull-in-flight detection: while a pull is pending or running, a
        // subtree scan must not infer deletions from "in DB but missing on
        // disk", otherwise in-flight downloads get marked deleted and the
        // source quarantines its only copy (see
        // "docs/design/ADR-20261105-scanner-incremental-paths.md").
        let pull_in_flight = {
            let pending = !self.pending_pulls.read().await.is_empty();
            let pulling = old_status == FolderStatus::Pulling;
            pending || pulling
        };
        if pull_in_flight {
            debug!(
                folder_id = %folder_id,
                changes = changes.len(),
                deletes = deletes.len(),
                "Pull in flight; subtree delete inference disabled for this incremental scan"
            );
        }

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
                    if pull_in_flight {
                        continue;
                    }
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

        for relative in deletes {
            // Truncating overwrites (shell `>`, editor saves) surface as
            // remove+create, so the same path appears in both changes and
            // deletes; when the file still exists the delete event is stale
            // and must be skipped, otherwise the deletion record would
            // overwrite the freshly scanned version and broadcast a phantom
            // deletion (see the module-level ADR pointer).
            if base_path.join(&relative).exists() {
                trace!(folder_id = %folder_id, path = %relative, "Stale delete event for existing file, skipping");
                continue;
            }
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

        changed_files = crate::scanner::detect_and_reorder_renames(changed_files);
        for file in &mut changed_files {
            if file.is_deleted() {
                file.blocks.clear();
            }
        }

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

        if !changed_files.is_empty() {
            self.events.publish(SyncEvent::LocalIndexUpdated {
                folder: folder_id.clone(),
                files: changed_files.clone(),
            });
        }

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

    /// Runs one pull round.
    ///
    /// Returns the round's statistics; `files_failed > 0` means some files
    /// were not pulled (failed files are never retried inline — the caller
    /// schedules a backoff retry). Skipped states (paused / non-syncable /
    /// already pulling / nothing to pull) return an all-zero stat.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`](crate::SyncError) when the needed-files check or the pull itself
    /// fails; the folder state is restored.
    pub async fn pull(&self) -> Result<PullStats> {
        if self.folder.paused {
            debug!(folder_id = %self.folder.id, "Folder is paused, skipping pull");
            return Ok(PullStats::default());
        }

        if !self.folder.folder_type.can_sync() {
            debug!(folder_id = %self.folder.id, "Folder type cannot sync, skipping pull");
            return Ok(PullStats::default());
        }

        let mut state = self.state.write().await;

        if state.status == FolderStatus::Pulling {
            trace!(folder_id = %self.folder.id, "Already pulling, skipping");
            return Ok(PullStats::default());
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

        let mut pending_files = {
            let mut pending = self.pending_pulls.write().await;
            std::mem::take(&mut *pending)
        };

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

            return Ok(PullStats::default());
        }

        info!(
            folder_id = %self.folder.id,
            file_count = needed_files.len(),
            "Pulling files"
        );

        {
            let mut state = self.state.write().await;
            state.need_files = needed_files.len();
        }

        let stats = match self.puller.pull_folder(&self.folder, needed_files).await {
            Ok(stats) => {
                info!(
                    folder_id = %self.folder.id,
                    succeeded = stats.files_succeeded,
                    failed = stats.files_failed,
                    "Pull completed"
                );

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

                stats
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
        };

        Ok(stats)
    }

    /// Handles a remote index: queues needed files and wakes the pull loop.
    ///
    /// # Errors
    ///
    /// This method currently never fails; the `Result` is reserved for future use.
    pub async fn handle_remote_index(&self, device: DeviceId, files: Vec<FileInfo>) -> Result<()> {
        debug!(
            folder_id = %self.folder.id,
            device = %device.short_id(),
            file_count = files.len(),
            "Handling remote index"
        );

        if !files.is_empty() {
            let mut pending = self.pending_pulls.write().await;
            for file in files {
                if !pending.iter().any(|f| f.name == file.name) {
                    pending.push(file);
                }
            }
            drop(pending);
        }

        // Wake the pull loop immediately. `notify_one` (not `notify_waiters`)
        // guarantees the loop receives the notification right after the
        // in-flight `pull()` finishes, without losing wakeups.
        self.pull_notify.notify_one();

        Ok(())
    }

    /// Replaces the folder configuration.
    pub async fn update_config(&mut self, config: Folder) {
        self.folder = config;
        self.events.publish(SyncEvent::FolderConfigUpdated {
            folder: self.folder.clone(),
        });
    }

    /// Pauses the folder.
    pub async fn pause(&self) {
        let mut state = self.state.write().await;
        state.status = FolderStatus::Idle;
        info!(folder_id = %self.folder.id, "Folder paused");
    }

    /// Resumes the folder.
    pub async fn resume(&self) {
        info!(folder_id = %self.folder.id, "Folder resumed");
    }

    /// Initialize folder state counters from the persisted database.
    /// Called when a folder is loaded at startup so that indices survive restarts
    /// without requiring an immediate full rescan.
    pub async fn init_state_from_db(&self) {
        match self.db.get_folder_files(&self.folder.id).await {
            Ok(files) => {
                let count = files.iter().filter(|f| !f.is_deleted()).count();
                let mut state = self.state.write().await;
                state.local_files = count;
                info!(folder_id = %self.folder.id, local_files = count, "Initialized folder state from database");
            }
            Err(e) => {
                warn!(folder_id = %self.folder.id, error = %e, "Failed to initialize folder state from database");
            }
        }
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
            file.version.increment(self.local_counter_id);
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
