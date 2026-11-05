//! Sync service.
//!
//! Main service implementation; manages all folder models and sync loops.
//!
//! T2.3 (2026-05-13): split by responsibility into four submodules. This
//! file keeps only the `SyncService` and `FolderTaskHandles` struct
//! definitions and the submodule declarations. Method implementations live in:
//!
//! - `lifecycle` — `new` / `with_*` builders / `start` / `stop` / `run` /
//!   accessors plus internal `init_folders`/`add_folder_internal`/`start_folder_loops`/
//!   `start_folder_internal` helpers
//! - `sync_manager` — `impl SyncManager for SyncService` (public CRUD + triggers)
//! - `network_bridge` — BEP network-layer callbacks (`handle_index` / `handle_block_request` etc.)
//! - `sync_model` — `impl syncthing_core::traits::SyncModel for SyncService` (FFI boundary)
//!
//! See `docs/drafts/RFC-001-service-split.md` for details.

use crate::database::LocalDatabase;
use crate::events::EventPublisher;
use crate::folder_model::FolderModel;
use crate::index_handler::IndexHandler;
use crate::orchestrator::FolderOrchestrator;
use crate::puller::{BlockSource, ConcurrencyPolicy};
use dashmap::DashMap;
use std::sync::Arc;
use syncthing_core::types::Config;
use syncthing_core::DeviceId;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

mod lifecycle;
mod network_bridge;
mod sync_manager;
mod sync_model;

/// Hook invoked on configuration changes (registered by the daemon; receives
/// the currently connected device list).
pub type RenegotiationHook = Arc<dyn Fn(Vec<DeviceId>) + Send + Sync>;

/// Main sync service.
pub struct SyncService {
    pub(super) config: RwLock<Config>,
    pub(super) folders: DashMap<String, Arc<FolderModel>>,
    pub(super) db: Arc<dyn LocalDatabase>,
    pub(super) events: EventPublisher,
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub(super) shutdown_rx: RwLock<tokio::sync::watch::Receiver<bool>>,
    pub(super) connected_devices: DashMap<DeviceId, ()>,
    pub(super) index_handler: IndexHandler,
    pub(super) block_source: RwLock<Option<Arc<dyn BlockSource>>>,
    pub(super) concurrency_policy: RwLock<Option<Arc<ConcurrencyPolicy>>>,
    pub(super) orchestrator: RwLock<Option<Arc<FolderOrchestrator>>>,
    /// Configuration-change renegotiation hook (registered by the daemon to
    /// trigger BEP session reconnects and re-exchange ClusterConfig).
    pub(super) renegotiation_hook: RwLock<Option<RenegotiationHook>>,
    /// Per-(device, folder) needed file count for completion tracking.
    pub(super) peer_sync_states: DashMap<(DeviceId, String), usize>,
    /// Per-folder task handles for individual start/stop control.
    pub(super) folder_tasks: DashMap<String, FolderTaskHandles>,
}

/// Per-folder async task handles.
pub(super) struct FolderTaskHandles {
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub(super) scan_handle: JoinHandle<()>,
    pub(super) pull_handle: JoinHandle<()>,
    pub(super) watcher_handle: JoinHandle<()>,
}

#[cfg(test)]
mod tests;
