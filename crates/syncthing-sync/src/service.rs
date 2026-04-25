//! 同步服务
//! 
//! 主服务实现，管理所有文件夹模型和同步循环

use crate::block_server;
use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{EventPublisher, EventSubscriber, SyncEvent};
use crate::folder_model::FolderModel;
use crate::index_handler::IndexHandler;
use crate::model::{SyncModel, SyncStats, FolderState};
use crate::puller::BlockSource;
use tokio::task::JoinHandle;
use syncthing_core::DeviceId;
use syncthing_core::types::{Config, FileInfo, Folder, Index, IndexUpdate};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// 同步服务
pub struct SyncService {
    config: RwLock<Config>,
    folders: DashMap<String, Arc<FolderModel>>,
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    shutdown_rx: RwLock<tokio::sync::watch::Receiver<bool>>,
    connected_devices: DashMap<DeviceId, ()>,
    index_handler: IndexHandler,
    block_source: RwLock<Option<Arc<dyn BlockSource>>>,
    /// Per-(device, folder) needed file count for completion tracking.
    peer_sync_states: DashMap<(DeviceId, String), usize>,
    /// Per-folder task handles for individual start/stop control.
    folder_tasks: DashMap<String, FolderTaskHandles>,
}

/// Per-folder async task handles.
struct FolderTaskHandles {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    scan_handle: JoinHandle<()>,
    pull_handle: JoinHandle<()>,
    watcher_handle: JoinHandle<()>,
}

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
        <Self as SyncModel>::start(self).await
    }

    /// 停止同步服务
    pub async fn stop(&self) -> Result<()> {
        <Self as SyncModel>::stop(self).await
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
    async fn init_folders(&self) -> Result<()> {
        let config = self.config.read().await;
        
        for folder_config in &config.folders {
            self.add_folder_internal(folder_config.clone()).await?;
        }

        info!(folder_count = self.folders.len(), "Folders initialized");
        Ok(())
    }

    /// 内部添加文件夹
    async fn add_folder_internal(&self, folder: Folder) -> Result<()> {
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
    async fn start_folder_loops(&self) {
        for entry in self.folders.iter() {
            let folder_id = entry.key().clone();
            if let Err(e) = self.start_folder_internal(&folder_id).await {
                warn!(folder_id = %folder_id, error = %e, "Failed to start folder loops");
            }
        }
    }

    /// 内部启动单个文件夹循环
    async fn start_folder_internal(&self, folder_id: &str) -> Result<()> {
        // 检查 folder 是否存在
        let folder_model = self.folders.get(folder_id)
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
            let model = model.clone();
            async move {
                model.start_watcher_loop(watcher_shutdown).await;
            }
        });

        self.folder_tasks.insert(folder_id.to_string(), FolderTaskHandles {
            shutdown_tx,
            scan_handle,
            pull_handle,
            watcher_handle,
        });

        info!(folder_id = %folder_id, "Folder loops started");
        Ok(())
    }
}

#[async_trait::async_trait]
impl SyncModel for SyncService {
    async fn get_config(&self) -> Result<Config> {
        Ok(self.config.read().await.clone())
    }

    async fn update_config(&self, config: Config) -> Result<()> {
        *self.config.write().await = config;
        Ok(())
    }

    async fn add_device(&self, device: syncthing_core::types::Device) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.devices.push(device);
        }
        Ok(())
    }

    async fn remove_device(&self, device_id: &DeviceId) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.devices.retain(|d| d.id != *device_id);
        }
        self.connected_devices.remove(device_id);
        Ok(())
    }

    async fn add_folder(&self, folder: Folder) -> Result<()> {
        // 添加到配置
        {
            let mut config = self.config.write().await;
            config.folders.push(folder.clone());
        }

        // 初始化文件夹
        self.add_folder_internal(folder).await?;

        Ok(())
    }

    async fn remove_folder(&self, folder_id: &str) -> Result<()> {
        // 从配置中移除
        {
            let mut config = self.config.write().await;
            config.folders.retain(|f| f.id != folder_id);
        }

        // 从运行时移除
        if self.folders.remove(folder_id).is_some() {
            info!(folder_id = %folder_id, "Folder removed");
        }

        Ok(())
    }

    async fn get_folder_state(&self, folder_id: &str) -> Result<FolderState> {
        match self.folders.get(folder_id) {
            Some(folder) => Ok(folder.state().await),
            None => Err(SyncError::FolderNotFound(folder_id.to_string())),
        }
    }

    async fn start(&self) -> Result<()> {
        info!("Starting sync service");

        // 初始化文件夹
        self.init_folders().await?;

        // 启动文件夹循环
        self.start_folder_loops().await;

        info!("Sync service started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Stopping sync service");

        // 发送全局关闭信号
        self.shutdown_tx.send(true).ok();

        // 收集所有 folder_id
        let folder_ids: Vec<String> = self.folder_tasks.iter().map(|e| e.key().clone()).collect();

        // 发送停止信号
        for folder_id in &folder_ids {
            if let Some(handles) = self.folder_tasks.get(folder_id) {
                handles.shutdown_tx.send(true).ok();
            }
        }

        // 等待所有任务完成
        for folder_id in folder_ids {
            if let Some((_, handles)) = self.folder_tasks.remove(&folder_id) {
                let _ = handles.scan_handle.await;
                let _ = handles.pull_handle.await;
                let _ = handles.watcher_handle.await;
            }
        }

        info!("Sync service stopped");
        Ok(())
    }

    async fn scan_folder(&self, folder_id: &str) -> Result<()> {
        match self.folders.get(folder_id) {
            Some(folder) => {
                folder.scan().await?;
                Ok(())
            }
            None => Err(SyncError::FolderNotFound(folder_id.to_string())),
        }
    }

    async fn pull_folder(&self, folder_id: &str) -> Result<()> {
        match self.folders.get(folder_id) {
            Some(folder) => {
                folder.pull().await?;
                Ok(())
            }
            None => Err(SyncError::FolderNotFound(folder_id.to_string())),
        }
    }

    async fn get_connected_devices(&self) -> Result<Vec<DeviceId>> {
        Ok(self.connected_devices.iter().map(|e: dashmap::mapref::multiple::RefMulti<'_, DeviceId, ()>| *e.key()).collect())
    }

    async fn connect_device(&self, device_id: DeviceId) -> Result<()> {
        self.connected_devices.insert(device_id, ());
        self.events.publish(SyncEvent::DeviceConnected { device: device_id });
        info!(device = %device_id.short_id(), "Device connected");
        Ok(())
    }

    async fn disconnect_device(&self, device_id: DeviceId) -> Result<()> {
        self.connected_devices.remove(&device_id);
        self.events.publish(SyncEvent::DeviceDisconnected {
            device: device_id,
            reason: "Manual disconnect".to_string(),
        });
        info!(device = %device_id.short_id(), "Device disconnected");
        Ok(())
    }

    fn subscribe_events(&self) -> EventSubscriber {
        self.events.subscribe()
    }

    async fn get_stats(&self) -> Result<SyncStats> {
        let mut stats = SyncStats::default();

        for entry in self.folders.iter() {
            let folder = entry.value();
            let state = folder.state().await;
            
            if let Ok(files) = self.db.get_folder_files(folder.id()).await {
                let folder_stats = crate::model::FolderStats {
                    files: state.local_files,
                    directories: files.iter().filter(|f| matches!(f.file_type, syncthing_core::types::FileType::Directory)).count(),
                    symlinks: files.iter().filter(|f| matches!(f.file_type, syncthing_core::types::FileType::Symlink)).count(),
                    total_bytes: files.iter().map(|f| f.size as u64).sum(),
                    deleted: files.iter().filter(|f| f.is_deleted()).count(),
                };
                
                let files_count = folder_stats.files;
                let bytes_count = folder_stats.total_bytes;
                stats.folders.insert(folder.id().to_string(), folder_stats);
                stats.total_files += files_count;
                stats.total_bytes += bytes_count;
            }
        }

        Ok(stats)
    }
}

impl SyncService {
    /// 处理接收到的索引消息（供网络层调用）
    pub async fn handle_index(&self, folder_id: &str, device: DeviceId, index: Index) -> Result<Vec<FileInfo>> {
        let folder_model = self.folders.get(folder_id)
            .ok_or_else(|| SyncError::FolderNotFound(folder_id.to_string()))?;

        let needed: Vec<syncthing_core::types::FileInfo> = self.index_handler.handle_index(folder_model.config(), device, index).await?;
        
        // 触发文件夹的远程索引处理
        folder_model.handle_remote_index(device, needed.clone()).await?;
        
        // Update peer sync state for completion tracking
        let key = (device, folder_id.to_string());
        self.peer_sync_states.insert(key, needed.len());
        
        Ok(needed)
    }

    /// 处理接收到的索引更新（供网络层调用）
    pub async fn handle_index_update(&self, folder_id: &str, device: DeviceId, update: IndexUpdate) -> Result<Vec<FileInfo>> {
        let folder_model = self.folders.get(folder_id)
            .ok_or_else(|| SyncError::FolderNotFound(folder_id.to_string()))?;

        let needed: Vec<syncthing_core::types::FileInfo> = self.index_handler.handle_index_update(folder_model.config(), device, update).await?;
        
        // 触发文件夹的远程索引处理
        folder_model.handle_remote_index(device, needed.clone()).await?;
        
        // Update peer sync state for completion tracking
        let key = (device, folder_id.to_string());
        self.peer_sync_states.insert(key, needed.len());
        
        Ok(needed)
    }

    /// 处理远程块请求（供网络层调用）
    pub async fn handle_block_request(
        &self,
        req: &bep_protocol::messages::Request,
    ) -> std::result::Result<Vec<u8>, block_server::BlockRequestError> {
        let config = self.config.read().await;
        let folder = config.folders.iter().find(|f| f.id == req.folder);
        let folder_path = match folder {
            Some(f) => std::path::PathBuf::from(&f.path),
            None => return Err(block_server::BlockRequestError::FolderNotFound),
        };
        drop(config);
        block_server::serve_block_request(&folder_path, req).await
    }

    /// 生成索引更新（供网络层调用）
    pub async fn generate_index_update(&self, folder_id: &str, since_sequence: u64) -> Result<Vec<FileInfo>> {
        self.index_handler.generate_index_update(folder_id, since_sequence).await
    }

    /// 获取所有文件夹ID
    pub fn get_folder_ids(&self) -> Vec<String> {
        self.folders.iter().map(|e| e.key().clone()).collect()
    }

    /// 获取文件夹模型
    pub fn get_folder(&self, folder_id: &str) -> Option<Arc<FolderModel>> {
        self.folders.get(folder_id).map(|f| f.clone())
    }

    /// 获取某个文件夹相对于某个设备的同步完成度（needed files 数量）
    pub fn get_folder_completion(&self, device_id: DeviceId, folder_id: &str) -> usize {
        self.peer_sync_states
            .get(&(device_id, folder_id.to_string()))
            .map(|v| *v)
            .unwrap_or(0)
    }
}

#[async_trait::async_trait]
impl syncthing_core::traits::SyncModel for SyncService {
    async fn start_folder(&self, folder: syncthing_core::FolderId) -> syncthing_core::Result<()> {
        let folder_id = folder.as_str();
        self.start_folder_internal(folder_id).await
            .map_err(|e| syncthing_core::SyncthingError::internal(e.to_string()))
    }

    async fn stop_folder(&self, folder: syncthing_core::FolderId) -> syncthing_core::Result<()> {
        let folder_id = folder.as_str();

        // 检查是否在运行
        let handles = self.folder_tasks.get(folder_id)
            .ok_or_else(|| syncthing_core::SyncthingError::internal(
                format!("Folder not running: {}", folder_id)
            ))?;

        // 发送停止信号
        handles.shutdown_tx.send(true).ok();
        drop(handles);

        // 等待任务完成并移除
        if let Some((_, handles)) = self.folder_tasks.remove(folder_id) {
            let _ = handles.scan_handle.await;
            let _ = handles.pull_handle.await;
            let _ = handles.watcher_handle.await;
        }

        info!(folder_id = %folder_id, "Folder stopped");
        Ok(())
    }

    async fn scan_folder(&self, folder: &syncthing_core::FolderId) -> syncthing_core::Result<()> {
        crate::model::SyncModel::scan_folder(self, folder.as_str()).await
            .map_err(|e| syncthing_core::SyncthingError::internal(e.to_string()))
    }

    async fn pull(&self, folder: &syncthing_core::FolderId) -> syncthing_core::Result<syncthing_core::traits::SyncResult> {
        crate::model::SyncModel::pull_folder(self, folder.as_str()).await
            .map_err(|e| syncthing_core::SyncthingError::internal(e.to_string()))?;
        Ok(syncthing_core::traits::SyncResult {
            files_processed: 0,
            bytes_transferred: 0,
            errors: vec![],
        })
    }

    async fn folder_status(&self, folder: &syncthing_core::FolderId) -> syncthing_core::Result<syncthing_core::traits::FolderStatus> {
        match self.get_folder(folder.as_str()) {
            Some(folder_model) => {
                let state = folder_model.state().await;
                let status = match state.status {
                    syncthing_core::types::FolderStatus::Idle
                    | syncthing_core::types::FolderStatus::ScanWaiting
                    | syncthing_core::types::FolderStatus::SyncWaiting
                    | syncthing_core::types::FolderStatus::Synced => {
                        syncthing_core::traits::FolderStatus::Idle
                    }
                    syncthing_core::types::FolderStatus::Scanning => {
                        syncthing_core::traits::FolderStatus::Scanning
                    }
                    syncthing_core::types::FolderStatus::Pulling
                    | syncthing_core::types::FolderStatus::Pushing => {
                        syncthing_core::traits::FolderStatus::Syncing { progress: 0.0 }
                    }
                    syncthing_core::types::FolderStatus::Paused => {
                        syncthing_core::traits::FolderStatus::Paused
                    }
                    syncthing_core::types::FolderStatus::Error => {
                        syncthing_core::traits::FolderStatus::Error {
                            message: "folder error".to_string(),
                        }
                    }
                };
                Ok(status)
            }
            None => Err(syncthing_core::SyncthingError::internal(format!(
                "folder not found: {}",
                folder
            ))),
        }
    }

    async fn folder_completion(
        &self,
        folder: &syncthing_core::FolderId,
        device: syncthing_core::DeviceId,
    ) -> syncthing_core::Result<u64> {
        let needed = self.get_folder_completion(device, folder.as_str());
        // Simple completion: 100% if needed == 0, else heuristic based on total files
        let total_files = self.db.get_folder_files(folder.as_str()).await
            .map(|v| v.len()).unwrap_or(0).max(needed);
        let completion = if total_files == 0 {
            100
        } else {
            (((total_files - needed) as f64 / total_files as f64) * 100.0) as u64
        };
        Ok(completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryDatabase;

    #[tokio::test]
    async fn test_service_creation() {
        let db = MemoryDatabase::new();
        let service = SyncService::new(db);
        
        assert!(service.get_folder_ids().is_empty());
    }

    #[tokio::test]
    async fn test_add_folder() {
        let db = MemoryDatabase::new();
        let service = SyncService::new(db);
        
        let folder = Folder::new("test", "/tmp/test");
        service.add_folder(folder).await.unwrap();
        
        assert_eq!(service.get_folder_ids().len(), 1);
    }

    #[tokio::test]
    async fn test_folder_not_found() {
        let db = MemoryDatabase::new();
        let service = SyncService::new(db);
        
        let result = service.get_folder_state("nonexistent").await;
        assert!(matches!(result, Err(SyncError::FolderNotFound(_))));
    }
}
