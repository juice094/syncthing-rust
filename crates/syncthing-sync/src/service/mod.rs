//! 同步服务
//!
//! 主服务实现，管理所有文件夹模型和同步循环。
//!
//! T2.3 (2026-05-13): 按职责拆分为四个子模块。本文件仅保留 `SyncService` 与
//! `FolderTaskHandles` 结构体定义、字段、以及子模块声明。所有方法实现在：
//!
//! - `lifecycle` — `new` / `with_*` builders / `start` / `stop` / `run` /
//!   accessors + 内部 `init_folders`/`add_folder_internal`/`start_folder_loops`/
//!   `start_folder_internal` 帮助函数
//! - `sync_manager` — `impl SyncManager for SyncService`（公开 CRUD + 触发）
//! - `network_bridge` — BEP 网络层回调（`handle_index` / `handle_block_request` 等）
//! - `sync_model` — `impl syncthing_core::traits::SyncModel for SyncService`（FFI 边界）
//!
//! 详见 `docs/drafts/RFC-001-service-split.md`。

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

/// 配置变更重协商钩子类型（daemon 注册，接收当前已连接设备列表）
pub type RenegotiationHook = Arc<dyn Fn(Vec<DeviceId>) + Send + Sync>;

/// 同步服务
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
    /// 配置变更重协商钩子（daemon 注册，用于触发 BEP 会话重连以重新交换 ClusterConfig）
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
