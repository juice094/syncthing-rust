//! SyncService construction & lifecycle.
//!
//! Extracted from `mod.rs` in T2.3 (see `docs/drafts/RFC-001-service-split.md`).
//! Holds: `new`, `with_*` builders, `start`/`stop`/`run`, accessors, and the
//! internal helpers `init_folders`, `add_folder_internal`, `start_folder_loops`,
//! and `start_folder_internal`. No business changes.

use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::EventPublisher;
use crate::folder_model::FolderModel;
use crate::index_handler::IndexHandler;
use crate::model::SyncManager;
use crate::orchestrator::{FolderOrchestrator, OrchestratorConfig};
use crate::puller::{BlockSource, ConcurrencyPolicy};
use crate::service::{FolderTaskHandles, SyncService};
use dashmap::DashMap;
use std::sync::Arc;
use syncthing_core::types::{Config, Folder};
use syncthing_core::DeviceId;
use tokio::sync::RwLock;
use tracing::{info, warn};

impl SyncService {
    /// Creates a new sync service.
    pub fn new(db: Arc<dyn LocalDatabase>) -> Self {
        let events = EventPublisher::new(1000);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let index_handler = IndexHandler::new(db.clone(), events.clone());

        Self {
            config: RwLock::new(Config::new()),
            folders: DashMap::new(),
            db,
            events,
            shutdown_tx,
            shutdown_rx: RwLock::new(shutdown_rx),
            connected_devices: DashMap::new(),
            index_handler,
            block_source: RwLock::new(None),
            concurrency_policy: RwLock::new(None),
            orchestrator: RwLock::new(None),
            renegotiation_hook: RwLock::new(None),
            folder_tasks: DashMap::new(),
            peer_sync_states: DashMap::new(),
        }
    }

    /// Sets the configuration (builder style).
    pub async fn with_config(self, config: Config) -> Self {
        if let Some(id) = config.local_device_id {
            self.index_handler.set_local_device_id(id);
        }
        *self.config.write().await = config;
        self
    }

    /// Sets the block source (synchronous builder).
    pub fn with_block_source(self, source: Arc<dyn BlockSource>) -> Self {
        *self.block_source.blocking_write() = Some(source);
        self
    }

    /// Sets the block source and propagates it to existing folder models.
    pub async fn set_block_source(&self, source: Arc<dyn BlockSource>) {
        *self.block_source.write().await = Some(source.clone());
        for entry in self.folders.iter() {
            entry.value().set_block_source(Some(source.clone()));
        }
    }

    /// Registers the configuration-change renegotiation hook.
    ///
    /// BEP only exchanges ClusterConfig at connection establishment; after a
    /// configuration change a disconnect/reconnect is required to re-exchange
    /// it (Go Syncthing behavior). The sync crate does not touch the network,
    /// so the daemon registers this hook to perform the actual reconnect.
    pub async fn set_renegotiation_hook(&self, hook: crate::service::RenegotiationHook) {
        *self.renegotiation_hook.write().await = Some(hook);
    }

    /// Fires the renegotiation hook with the current connected device list.
    pub(super) async fn fire_renegotiation_hook(&self) {
        let hook = self.renegotiation_hook.read().await.clone();
        if let Some(hook) = hook {
            let devices: Vec<DeviceId> = self.connected_devices.iter().map(|e| *e.key()).collect();
            if !devices.is_empty() {
                hook(devices);
            }
        }
    }

    /// Sets the shared concurrency policy (builder style).
    pub fn with_concurrency_policy(self, policy: Arc<ConcurrencyPolicy>) -> Self {
        *self.concurrency_policy.blocking_write() = Some(policy);
        self
    }

    /// Sets the shared concurrency policy and pushes it to existing folder models.
    pub async fn set_concurrency_policy(&self, policy: Arc<ConcurrencyPolicy>) {
        *self.concurrency_policy.write().await = Some(policy.clone());
        for entry in self.folders.iter() {
            entry.value().set_concurrency_policy(policy.clone());
        }
    }

    /// Sets the folder orchestrator (builder style).
    pub fn with_orchestrator(self, orchestrator: Arc<FolderOrchestrator>) -> Self {
        *self.orchestrator.blocking_write() = Some(orchestrator);
        self
    }

    /// Sets the folder orchestrator and pushes it to existing folder models.
    pub async fn set_orchestrator(&self, orchestrator: Arc<FolderOrchestrator>) {
        *self.orchestrator.write().await = Some(orchestrator.clone());
        for entry in self.folders.iter() {
            entry.value().set_orchestrator(orchestrator.clone());
        }
    }

    /// Updates the orchestrator configuration (when an orchestrator is set).
    pub fn set_orchestrator_config(&self, config: OrchestratorConfig) {
        if let Ok(guard) = self.orchestrator.try_read() {
            if let Some(ref orch) = *guard {
                orch.set_config(config);
            }
        }
    }

    /// Starts the sync service.
    pub async fn start(&self) -> Result<()> {
        <Self as SyncManager>::start(self).await
    }

    /// Stops the sync service.
    pub async fn stop(&self) -> Result<()> {
        <Self as SyncManager>::stop(self).await
    }

    /// Runs the sync service until the shutdown signal fires.
    pub async fn run(&self) -> Result<()> {
        self.start().await?;
        let mut shutdown_rx = self.shutdown_rx.read().await.clone();
        while !*shutdown_rx.borrow_and_update() {
            if shutdown_rx.changed().await.is_err() {
                break;
            }
        }
        self.stop().await
    }

    /// Returns a handle to the local database.
    pub fn db(&self) -> Arc<dyn LocalDatabase> {
        self.db.clone()
    }

    /// Returns the event publisher.
    pub fn events(&self) -> &EventPublisher {
        &self.events
    }

    /// Initializes all configured folders.
    pub(super) async fn init_folders(&self) -> Result<()> {
        let config = self.config.read().await;

        for folder_config in &config.folders {
            self.add_folder_internal(folder_config.clone()).await?;
        }

        info!(folder_count = self.folders.len(), "Folders initialized");
        Ok(())
    }

    /// Adds a single folder (runtime + persistence).
    pub(super) async fn add_folder_internal(&self, folder: Folder) -> Result<()> {
        let folder_id = folder.id.clone();

        if self.folders.contains_key(&folder_id) {
            warn!(folder_id = %folder_id, "Folder already exists");
            return Ok(());
        }

        self.db.update_folder(folder.clone()).await?;

        let block_source = self.block_source.read().await.clone();
        let concurrency_policy = self.concurrency_policy.read().await.clone();
        let orchestrator = self.orchestrator.read().await.clone();
        // The version-vector local counter ID must come from the real device
        // ID: hardcoding it would misclassify concurrent offline edits on
        // both sides as linear history, silently letting the deleter
        // overwrite the peer's modifications.
        let local_counter_id = self
            .config
            .read()
            .await
            .local_device_id
            .map(|id| id.counter_id())
            .unwrap_or_else(|| {
                warn!(
                    folder_id = %folder_id,
                    "config.local_device_id missing: version vector counter id falls back to 0; \
                     concurrent-modification detection will not work across nodes with the same fallback"
                );
                0
            });
        let folder_model = Arc::new(
            FolderModel::new(
                folder,
                self.db.clone(),
                self.events.clone(),
                block_source,
                local_counter_id,
            )
            .with_concurrency_policy(concurrency_policy)
            .with_orchestrator(orchestrator),
        );

        self.folders.insert(folder_id.clone(), folder_model.clone());
        // Restore folder state counters from the persisted database so index
        // statistics are immediately visible after a restart.
        folder_model.init_state_from_db().await;
        info!(folder_id = %folder_id, "Folder added");

        Ok(())
    }

    /// Starts the loops of all folders.
    pub(super) async fn start_folder_loops(&self) {
        for entry in self.folders.iter() {
            let folder_id = entry.key().clone();
            if let Err(e) = self.start_folder_internal(&folder_id).await {
                warn!(folder_id = %folder_id, error = %e, "Failed to start folder loops");
            }
        }
    }

    /// Starts the loops of a single folder (idempotent).
    pub(super) async fn start_folder_internal(&self, folder_id: &str) -> Result<()> {
        let folder_model = self
            .folders
            .get(folder_id)
            .ok_or_else(|| SyncError::FolderNotFound(folder_id.to_string()))?;

        if self.folder_tasks.contains_key(folder_id) {
            warn!(folder_id = %folder_id, "Folder already running, skipping start_folder_internal");
            return Ok(());
        }

        let (shutdown_tx, scan_shutdown) = tokio::sync::watch::channel(false);
        let pull_shutdown = shutdown_tx.subscribe();
        let watcher_shutdown = shutdown_tx.subscribe();

        let model = folder_model.clone();
        let scan_handle = tokio::spawn({
            let model = model.clone();
            async move {
                model.start_scan_loop(scan_shutdown).await;
            }
        });

        let pull_handle = tokio::spawn({
            let model = model.clone();
            async move {
                model.start_pull_loop(pull_shutdown).await;
            }
        });

        let watcher_handle = tokio::spawn({
            let model = model;
            async move {
                model.start_watcher_loop(watcher_shutdown).await;
            }
        });

        self.folder_tasks.insert(
            folder_id.to_string(),
            FolderTaskHandles {
                shutdown_tx,
                scan_handle,
                pull_handle,
                watcher_handle,
            },
        );

        info!(folder_id = %folder_id, "Folder loops started");
        Ok(())
    }
}
