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
    /// 创建新的同步服务
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

    /// 使用配置创建服务
    pub async fn with_config(self, config: Config) -> Self {
        *self.config.write().await = config;
        self
    }

    /// 设置块数据源（同步构建器）
    pub fn with_block_source(self, source: Arc<dyn BlockSource>) -> Self {
        *self.block_source.blocking_write() = Some(source);
        self
    }

    /// 设置块数据源
    pub async fn set_block_source(&self, source: Arc<dyn BlockSource>) {
        *self.block_source.write().await = Some(source.clone());
        // 传播到已存在的 FolderModel（此前仅创建时读取，add_folder 后设置不生效）
        for entry in self.folders.iter() {
            entry.value().set_block_source(Some(source.clone()));
        }
    }

    /// 注册配置变更重协商钩子。
    ///
    /// BEP 只在连接建立时交换 ClusterConfig；配置变更后必须断开重连
    /// 才会重新交换（Go Syncthing 行为）。sync crate 不触碰网络，
    /// 由 daemon 注册本钩子执行实际的 reconnect。
    pub async fn set_renegotiation_hook(&self, hook: crate::service::RenegotiationHook) {
        *self.renegotiation_hook.write().await = Some(hook);
    }

    /// 配置变更后调用：以当前已连接设备列表触发重协商钩子
    pub(super) async fn fire_renegotiation_hook(&self) {
        let hook = self.renegotiation_hook.read().await.clone();
        if let Some(hook) = hook {
            let devices: Vec<DeviceId> = self.connected_devices.iter().map(|e| *e.key()).collect();
            if !devices.is_empty() {
                hook(devices);
            }
        }
    }

    /// 设置共享并发策略（构建器模式）
    pub fn with_concurrency_policy(self, policy: Arc<ConcurrencyPolicy>) -> Self {
        *self.concurrency_policy.blocking_write() = Some(policy);
        self
    }

    /// 设置共享并发策略并动态下发到所有已存在的文件夹模型
    pub async fn set_concurrency_policy(&self, policy: Arc<ConcurrencyPolicy>) {
        *self.concurrency_policy.write().await = Some(policy.clone());
        for entry in self.folders.iter() {
            entry.value().set_concurrency_policy(policy.clone());
        }
    }

    /// 设置文件夹编排器（构建器模式）
    pub fn with_orchestrator(self, orchestrator: Arc<FolderOrchestrator>) -> Self {
        *self.orchestrator.blocking_write() = Some(orchestrator);
        self
    }

    /// 设置文件夹编排器并动态下发到所有已存在的文件夹模型
    pub async fn set_orchestrator(&self, orchestrator: Arc<FolderOrchestrator>) {
        *self.orchestrator.write().await = Some(orchestrator.clone());
        for entry in self.folders.iter() {
            entry.value().set_orchestrator(orchestrator.clone());
        }
    }

    /// 设置编排器配置（当编排器已存在时）
    pub fn set_orchestrator_config(&self, config: OrchestratorConfig) {
        if let Ok(guard) = self.orchestrator.try_read() {
            if let Some(ref orch) = *guard {
                orch.set_config(config);
            }
        }
    }

    /// 启动同步服务
    pub async fn start(&self) -> Result<()> {
        <Self as SyncManager>::start(self).await
    }

    /// 停止同步服务
    pub async fn stop(&self) -> Result<()> {
        <Self as SyncManager>::stop(self).await
    }

    /// 运行同步服务直到收到关闭信号
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

    /// 获取数据库引用
    pub fn db(&self) -> Arc<dyn LocalDatabase> {
        self.db.clone()
    }

    /// 获取事件发布者
    pub fn events(&self) -> &EventPublisher {
        &self.events
    }

    /// 初始化文件夹
    pub(super) async fn init_folders(&self) -> Result<()> {
        let config = self.config.read().await;

        for folder_config in &config.folders {
            self.add_folder_internal(folder_config.clone()).await?;
        }

        info!(folder_count = self.folders.len(), "Folders initialized");
        Ok(())
    }

    /// 内部添加文件夹
    pub(super) async fn add_folder_internal(&self, folder: Folder) -> Result<()> {
        let folder_id = folder.id.clone();

        // 检查是否已存在
        if self.folders.contains_key(&folder_id) {
            warn!(folder_id = %folder_id, "Folder already exists");
            return Ok(());
        }

        // 更新数据库中的文件夹配置
        self.db.update_folder(folder.clone()).await?;

        // 创建文件夹模型
        let block_source = self.block_source.read().await.clone();
        let concurrency_policy = self.concurrency_policy.read().await.clone();
        let orchestrator = self.orchestrator.read().await.clone();
        // 版本向量的本地计数器 ID 必须来自真实设备 ID（硬编码 1 会导致
        // 双端离线并发修改被误判为线性历史，删除方静默覆盖对端修改）
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

        self.folders.insert(folder_id.clone(), folder_model);
        info!(folder_id = %folder_id, "Folder added");

        Ok(())
    }

    /// 启动所有文件夹循环
    pub(super) async fn start_folder_loops(&self) {
        for entry in self.folders.iter() {
            let folder_id = entry.key().clone();
            if let Err(e) = self.start_folder_internal(&folder_id).await {
                warn!(folder_id = %folder_id, error = %e, "Failed to start folder loops");
            }
        }
    }

    /// 内部启动单个文件夹循环
    pub(super) async fn start_folder_internal(&self, folder_id: &str) -> Result<()> {
        // 检查 folder 是否存在
        let folder_model = self
            .folders
            .get(folder_id)
            .ok_or_else(|| SyncError::FolderNotFound(folder_id.to_string()))?;

        // 如果已经在运行，直接返回
        if self.folder_tasks.contains_key(folder_id) {
            warn!(folder_id = %folder_id, "Folder already running, skipping start_folder_internal");
            return Ok(());
        }

        // 创建独立的 shutdown channel
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
