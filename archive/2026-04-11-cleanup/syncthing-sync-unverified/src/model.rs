//! Module: syncthing-sync
//! Worker: Agent-C
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! 同步模型核心实现
//!
//! 该模块实现 `SyncModel` trait，是同步引擎的核心组件，负责：
//! - 管理文件夹同步状态机
//! - 协调 puller 和 pusher 的工作
//! - 处理传入的连接
//! - 维护文件夹级别的同步状态

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info, trace, warn};

use syncthing_core::traits::{
    BepConnection, BepMessage, BlockStore, ConfigStore, EventPublisher, FileSystem, SyncModel,
    SyncResult, FolderStatus,
};
use syncthing_core::types::{DeviceId, FolderConfig, FolderId};
use syncthing_core::Result;

use crate::conflict::ConflictManager;
use crate::index::IndexManager;
use crate::puller::Puller;
use crate::pusher::Pusher;

/// 文件夹同步状态机
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// 空闲状态
    Idle,
    /// 正在扫描
    Scanning,
    /// 正在同步（拉取）
    Pulling,
    /// 正在推送
    Pushing,
    /// 错误状态
    Error,
    /// 已暂停
    Paused,
}

impl std::fmt::Display for SyncState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncState::Idle => write!(f, "idle"),
            SyncState::Scanning => write!(f, "scanning"),
            SyncState::Pulling => write!(f, "syncing"),
            SyncState::Pushing => write!(f, "syncing"),
            SyncState::Error => write!(f, "error"),
            SyncState::Paused => write!(f, "paused"),
        }
    }
}

/// 文件夹同步上下文
pub struct FolderContext {
    /// 文件夹配置
    config: FolderConfig,
    /// 文件夹路径
    path: PathBuf,
    /// 同步状态
    state: Arc<RwLock<SyncState>>,
    /// 索引管理器
    index_manager: Arc<IndexManager>,
    /// 拉取器
    puller: Arc<Puller>,
    /// 推送器
    pusher: Arc<Pusher>,
    /// 冲突管理器
    #[allow(dead_code)]
    conflict_manager: Arc<Mutex<ConflictManager>>,
    /// 当前进度（0.0 - 1.0）
    progress: Arc<RwLock<f64>>,
    /// 最后错误信息
    last_error: Arc<RwLock<Option<String>>>,
}

impl FolderContext {
    /// 创建新的文件夹上下文
    #[allow(clippy::too_many_arguments)]
    fn new(
        config: FolderConfig,
        path: PathBuf,
        index_manager: Arc<IndexManager>,
        puller: Arc<Puller>,
        pusher: Arc<Pusher>,
        conflict_manager: Arc<Mutex<ConflictManager>>,
    ) -> Self {
        Self {
            config,
            path,
            state: Arc::new(RwLock::new(SyncState::Idle)),
            index_manager,
            puller,
            pusher,
            conflict_manager,
            progress: Arc::new(RwLock::new(0.0)),
            last_error: Arc::new(RwLock::new(None)),
        }
    }

    /// 获取当前状态
    async fn get_state(&self) -> SyncState {
        *self.state.read().await
    }

    /// 设置状态
    async fn set_state(&self, state: SyncState) {
        let mut current = self.state.write().await;
        *current = state;
    }

    /// 获取进度
    async fn get_progress(&self) -> f64 {
        *self.progress.read().await
    }

    /// 设置进度
    async fn set_progress(&self, progress: f64) {
        let mut current = self.progress.write().await;
        *current = progress.clamp(0.0, 1.0);
    }

    /// 设置错误
    async fn set_error(&self, error: Option<String>) {
        let mut current = self.last_error.write().await;
        *current = error;
    }
}

impl std::fmt::Debug for FolderContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FolderContext")
            .field("config", &self.config)
            .field("path", &self.path)
            .finish()
    }
}

/// 同步引擎实现
pub struct SyncEngine {
    /// 本地设备 ID
    local_device: DeviceId,
    /// 文件夹上下文映射
    folders: Arc<RwLock<HashMap<FolderId, Arc<FolderContext>>>>,
    /// 块存储
    block_store: Arc<dyn BlockStore>,
    /// 文件系统
    file_system: Arc<dyn FileSystem>,
    /// 配置存储
    #[allow(dead_code)]
    config_store: Arc<dyn ConfigStore>,
    /// 事件发布器
    #[allow(dead_code)]
    event_publisher: Option<Arc<dyn EventPublisher>>,
    /// 活动连接处理器
    connection_handlers: Arc<Mutex<Vec<JoinHandle<Result<()>>>>>,
    /// 推送器映射（按文件夹）
    pushers: Arc<RwLock<HashMap<FolderId, Arc<Pusher>>>>,
}

impl SyncEngine {
    /// 创建新的同步引擎
    pub fn new(
        local_device: DeviceId,
        block_store: Arc<dyn BlockStore>,
        file_system: Arc<dyn FileSystem>,
        config_store: Arc<dyn ConfigStore>,
        event_publisher: Option<Arc<dyn EventPublisher>>,
    ) -> Self {
        Self {
            local_device,
            folders: Arc::new(RwLock::new(HashMap::new())),
            block_store,
            file_system,
            config_store,
            event_publisher,
            connection_handlers: Arc::new(Mutex::new(Vec::new())),
            pushers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 添加文件夹
    ///
    /// 在启动同步之前配置文件夹
    pub async fn add_folder(&self,
        config: FolderConfig,
    ) -> Result<()> {
        info!("添加文件夹: {} at {:?}", config.id, config.path);

        let folder_id = config.id.clone();
        let folder_path = config.path.clone();

        // 创建索引管理器
        let index_manager = Arc::new(IndexManager::new(
            folder_id.clone(),
            self.local_device,
            self.block_store.clone(),
        ));

        // 加载本地索引
        index_manager.load_local_index().await?;

        // 创建拉取器
        let puller = Arc::new(Puller::new(
            folder_id.clone(),
            folder_path.clone(),
            self.local_device,
            index_manager.clone(),
            self.block_store.clone(),
            self.file_system.clone(),
            4, // 最大并发下载数
        ));

        // 创建推送器
        let pusher = Arc::new(Pusher::new(
            folder_id.clone(),
            folder_path.clone(),
            self.local_device,
            index_manager.clone(),
            self.block_store.clone(),
            self.file_system.clone(),
        ));

        // 创建冲突管理器
        let conflict_manager = Arc::new(Mutex::new(ConflictManager::new()));

        // 创建文件夹上下文
        let context = Arc::new(FolderContext::new(
            config,
            folder_path,
            index_manager,
            puller,
            pusher.clone(),
            conflict_manager,
        ));

        // 存储文件夹上下文
        {
            let mut folders = self.folders.write().await;
            folders.insert(folder_id.clone(), context);
        }

        // 存储推送器
        {
            let mut pushers = self.pushers.write().await;
            pushers.insert(folder_id, pusher);
        }

        Ok(())
    }

    /// 移除文件夹
    pub async fn remove_folder(&self, folder: &FolderId) -> Result<()> {
        info!("移除文件夹: {}", folder);

        // 先停止同步
        self.stop_folder(folder.clone()).await?;

        // 移除文件夹上下文
        {
            let mut folders = self.folders.write().await;
            folders.remove(folder);
        }

        // 移除推送器
        {
            let mut pushers = self.pushers.write().await;
            pushers.remove(folder);
        }

        Ok(())
    }

    /// 扫描文件夹
    ///
    /// 扫描本地文件并更新索引
    pub async fn scan_folder(&self, folder: &FolderId) -> Result<()> {
        info!("扫描文件夹: {}", folder);

        let context = self
            .get_folder_context(folder)
            .await
            .ok_or_else(|| syncthing_core::SyncthingError::FolderNotFound(folder.clone()))?;

        // 设置状态为扫描中
        context.set_state(SyncState::Scanning).await;
        context.set_progress(0.0).await;

        // 执行扫描
        let path = context.path.clone();
        let files = self.file_system.scan_directory(&path).await?;

        info!("扫描完成: {} 个文件", files.len());

        // 更新索引
        context.index_manager.replace_local_index(files).await?;

        // 广播索引更新到所有连接的设备
        let all_files = context.index_manager.get_all_local_files().await;
        context.pusher.broadcast_index_update(all_files).await?;

        // 恢复空闲状态
        context.set_state(SyncState::Idle).await;
        context.set_progress(1.0).await;

        Ok(())
    }

    #[allow(dead_code)]
    /// 获取文件夹上下文
    async fn get_folder_context(&self,
        folder: &FolderId,
    ) -> Option<Arc<FolderContext>> {
        let folders = self.folders.read().await;
        folders.get(folder).cloned()
    }

    #[allow(dead_code)]
    /// 处理连接的消息循环（已由 SyncEngineHandle 替代使用）
    async fn handle_connection_loop(
        &self,
        device: DeviceId,
        mut connection: Box<dyn BepConnection>,
    ) -> Result<()> {
        info!("开始处理连接: device={}", device.short_id());

        // 发送本地索引到新连接的设备
        {
            let folders = self.folders.read().await;
            for (folder_id, context) in folders.iter() {
                if let Err(e) = context.pusher.send_index(device).await {
                    warn!("发送索引失败: folder={}, error={}", folder_id, e);
                }
            }
        }

        // 运行消息处理循环
        loop {
            match connection.recv_message().await? {
                Some(msg) => {
                    trace!("收到消息: {:?}", msg);
                    // 根据消息类型处理
                    match &msg {
                        BepMessage::Index { folder, .. } | BepMessage::IndexUpdate { folder, .. } => {
                            // 更新索引
                            let folders = self.folders.read().await;
                            if let Some(context) = folders.get(folder) {
                                if let Err(e) = context.pusher.handle_message(device, msg).await {
                                    warn!("处理消息失败: {}", e);
                                }
                            }
                        }
                        _ => {
                            // 其他消息类型
                        }
                    }
                }
                None => {
                    info!("连接关闭: device={}", device.short_id());
                    break;
                }
            }
        }

        info!("连接处理结束: device={}", device.short_id());
        Ok(())
    }
}

#[async_trait]
impl SyncModel for SyncEngine {
    async fn start_folder(&self, folder: FolderId) -> Result<()> {
        info!("启动文件夹同步: {}", folder);

        let context = self
            .get_folder_context(&folder)
            .await
            .ok_or_else(|| syncthing_core::SyncthingError::FolderNotFound(folder.clone()))?;

        let current_state = context.get_state().await;
        match current_state {
            SyncState::Idle | SyncState::Error => {
                context.set_state(SyncState::Idle).await;
                context.set_error(None).await;

                // 执行初始扫描
                if let Err(e) = self.scan_folder(&folder).await {
                    error!("初始扫描失败: {}", e);
                    context.set_state(SyncState::Error).await;
                    context.set_error(Some(e.to_string())).await;
                    return Err(e);
                }

                info!("文件夹同步已启动: {}", folder);
                Ok(())
            }
            SyncState::Paused => {
                context.set_state(SyncState::Idle).await;
                info!("文件夹同步已恢复: {}", folder);
                Ok(())
            }
            _ => {
                warn!("文件夹 {} 已经在运行中", folder);
                Ok(())
            }
        }
    }

    async fn stop_folder(&self, folder: FolderId) -> Result<()> {
        info!("停止文件夹同步: {}", folder);

        let context = self
            .get_folder_context(&folder)
            .await
            .ok_or_else(|| syncthing_core::SyncthingError::FolderNotFound(folder.clone()))?;

        context.set_state(SyncState::Idle).await;
        context.set_progress(0.0).await;

        info!("文件夹同步已停止: {}", folder);
        Ok(())
    }

    async fn pull(&self, folder: &FolderId) -> Result<SyncResult> {
        info!("执行拉取: folder={}", folder);

        let context = self
            .get_folder_context(folder)
            .await
            .ok_or_else(|| syncthing_core::SyncthingError::FolderNotFound(folder.clone()))?;

        // 检查状态
        let current_state = context.get_state().await;
        match current_state {
            SyncState::Paused => {
                return Ok(SyncResult {
                    files_processed: 0,
                    bytes_transferred: 0,
                    errors: vec!["文件夹已暂停".to_string()],
                });
            }
            SyncState::Pulling => {
                warn!("拉取已在进行中: {}", folder);
                return Ok(SyncResult {
                    files_processed: 0,
                    bytes_transferred: 0,
                    errors: vec!["拉取已在进行中".to_string()],
                });
            }
            _ => {}
        }

        // 设置状态为拉取中
        context.set_state(SyncState::Pulling).await;
        context.set_progress(0.0).await;

        // 执行拉取
        let result = context.puller.pull().await;

        // 恢复状态
        match &result {
            Ok(_) => {
                context.set_state(SyncState::Idle).await;
                context.set_progress(1.0).await;
            }
            Err(e) => {
                error!("拉取失败: {}", e);
                context.set_state(SyncState::Error).await;
                context.set_error(Some(e.to_string())).await;
            }
        }

        // 转换为 SyncResult
        let pull_result = result?;
        Ok(SyncResult {
            files_processed: pull_result.files_processed,
            bytes_transferred: pull_result.bytes_transferred,
            errors: pull_result.errors,
        })
    }

    async fn folder_status(&self, folder: &FolderId) -> Result<FolderStatus> {
        let context = match self.get_folder_context(folder).await {
            Some(ctx) => ctx,
            None => {
                return Ok(FolderStatus::Error {
                    message: format!("文件夹 {} 不存在", folder),
                });
            }
        };

        let state = context.get_state().await;
        let progress = context.get_progress().await;
        let error = context.last_error.read().await.clone();

        let status = match state {
            SyncState::Idle => FolderStatus::Idle,
            SyncState::Scanning => FolderStatus::Scanning,
            SyncState::Pulling | SyncState::Pushing => FolderStatus::Syncing { progress },
            SyncState::Error => FolderStatus::Error {
                message: error.unwrap_or_else(|| "未知错误".to_string()),
            },
            SyncState::Paused => FolderStatus::Paused,
        };

        Ok(status)
    }

    async fn scan_folder(&self, folder: &FolderId) -> Result<()> {
        self.scan_folder(folder).await
    }

    async fn handle_connection(
        &self,
        conn: Box<dyn BepConnection>,
    ) -> Result<()> {
        let device = conn.remote_device();
        info!("处理新连接: device={}", device.short_id());

        // 在后台任务中处理连接
        let engine = Arc::new(self.clone_as_handle());
        let handle = tokio::spawn(async move {
            engine.handle_connection_loop(device, conn).await
        });

        // 保存任务句柄，并清理已完成的任务以避免内存泄漏
        {
            let mut handlers = self.connection_handlers.lock().await;
            handlers.retain(|h| !h.is_finished());
            handlers.push(handle);
        }

        Ok(())
    }
}

/// 同步引擎句柄（用于克隆）
#[derive(Clone)]
pub struct SyncEngineHandle {
    #[allow(dead_code)]
    local_device: DeviceId,
    folders: Arc<RwLock<HashMap<FolderId, Arc<FolderContext>>>>,
    #[allow(dead_code)]
    block_store: Arc<dyn BlockStore>,
    #[allow(dead_code)]
    file_system: Arc<dyn FileSystem>,
    #[allow(dead_code)]
    pushers: Arc<RwLock<HashMap<FolderId, Arc<Pusher>>>>,
}

impl SyncEngine {
    /// 克隆为句柄（用于在任务中使用）
    fn clone_as_handle(&self) -> SyncEngineHandle {
        SyncEngineHandle {
            local_device: self.local_device,
            folders: self.folders.clone(),
            block_store: self.block_store.clone(),
            file_system: self.file_system.clone(),
            pushers: self.pushers.clone(),
        }
    }
}

impl SyncEngineHandle {
    #[allow(dead_code)]
    /// 获取文件夹上下文
    async fn get_folder_context(
        &self,
        folder: &FolderId,
    ) -> Option<Arc<FolderContext>> {
        let folders = self.folders.read().await;
        folders.get(folder).cloned()
    }

    /// 处理连接的消息循环
    async fn handle_connection_loop(
        &self,
        device: DeviceId,
        mut connection: Box<dyn BepConnection>,
    ) -> Result<()> {
        info!("开始处理连接: device={}", device.short_id());

        // 发送本地索引到新连接的设备
        {
            let folders = self.folders.read().await;
            for (folder_id, context) in folders.iter() {
                if let Err(e) = context.pusher.send_index(device).await {
                    warn!("发送索引失败: folder={}, error={}", folder_id, e);
                }
            }
        }

        // 运行消息处理循环
        loop {
            match connection.recv_message().await? {
                Some(msg) => {
                    trace!("收到消息: {:?}", msg);
                    // 根据消息类型处理
                    match &msg {
                        BepMessage::Index { folder, .. } | BepMessage::IndexUpdate { folder, .. } => {
                            // 更新索引
                            let folders = self.folders.read().await;
                            if let Some(context) = folders.get(folder) {
                                if let Err(e) = context.pusher.handle_message(device, msg).await {
                                    warn!("处理消息失败: {}", e);
                                }
                            }
                        }
                        _ => {
                            // 其他消息类型
                        }
                    }
                }
                None => {
                    info!("连接关闭: device={}", device.short_id());
                    break;
                }
            }
        }

        info!("连接处理结束: device={}", device.short_id());
        Ok(())
    }
}

/// 同步配置
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// 最大并发下载数
    pub max_concurrent_downloads: usize,
    /// 最大并发上传数
    pub max_concurrent_uploads: usize,
    /// 块大小（字节）
    pub block_size: usize,
    /// 是否自动拉取
    pub auto_pull: bool,
    /// 是否自动扫描
    pub auto_scan: bool,
    /// 扫描间隔（秒）
    pub scan_interval_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 4,
            max_concurrent_uploads: 4,
            block_size: 128 * 1024, // 128KB
            auto_pull: true,
            auto_scan: true,
            scan_interval_secs: 3600,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::DeviceId;

    #[allow(dead_code)]
    fn create_test_device(id: u8) -> DeviceId {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        DeviceId::from_bytes(bytes)
    }

    #[test]
    fn test_sync_state_display() {
        assert_eq!(SyncState::Idle.to_string(), "idle");
        assert_eq!(SyncState::Scanning.to_string(), "scanning");
        assert_eq!(SyncState::Pulling.to_string(), "syncing");
        assert_eq!(SyncState::Error.to_string(), "error");
        assert_eq!(SyncState::Paused.to_string(), "paused");
    }

    #[test]
    fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert_eq!(config.max_concurrent_downloads, 4);
        assert_eq!(config.max_concurrent_uploads, 4);
        assert_eq!(config.block_size, 128 * 1024);
        assert!(config.auto_pull);
        assert!(config.auto_scan);
        assert_eq!(config.scan_interval_secs, 3600);
    }
}
