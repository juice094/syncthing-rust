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
use crate::puller::BlockSource;
use crate::service::{FolderTaskHandles, SyncService};
use dashmap::DashMap;
use std::sync::Arc;
use syncthing_core::types::{Config, Folder};
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
        *self.block_source.write().await = Some(source);
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
        let folder_model = Arc::new(FolderModel::new(
            folder,
            self.db.clone(),
            self.events.clone(),
            block_source,
        ));

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
