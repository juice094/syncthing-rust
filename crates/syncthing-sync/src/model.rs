//! 同步模型 trait 和状态定义

use crate::error::Result;
use crate::events::EventSubscriber;
use syncthing_core::DeviceId;
use syncthing_core::types::{Config, FileInfo, Folder, FolderStatus};
use async_trait::async_trait;
use std::collections::HashMap;

/// 同步模型 trait
#[async_trait]
pub trait SyncModel: Send + Sync {
    /// 获取配置
    async fn get_config(&self) -> Result<Config>;

    /// 更新配置
    async fn update_config(&self, config: Config) -> Result<()>;

    /// 添加设备
    async fn add_device(&self, device: syncthing_core::types::Device) -> Result<()>;

    /// 移除设备
    async fn remove_device(&self, device_id: &DeviceId) -> Result<()>;

    /// 添加文件夹
    async fn add_folder(&self, folder: Folder) -> Result<()>;

    /// 移除文件夹
    async fn remove_folder(&self, folder_id: &str) -> Result<()>;

    /// 获取文件夹状态
    async fn get_folder_state(&self, folder_id: &str) -> Result<FolderState>;

    /// 启动同步
    async fn start(&self) -> Result<()>;

    /// 停止同步
    async fn stop(&self) -> Result<()>;

    /// 触发文件夹扫描
    async fn scan_folder(&self, folder_id: &str) -> Result<()>;

    /// 触发文件夹拉取
    async fn pull_folder(&self, folder_id: &str) -> Result<()>;

    /// 获取连接的设备列表
    async fn get_connected_devices(&self) -> Result<Vec<DeviceId>>;

    /// 连接到设备
    async fn connect_device(&self, device_id: DeviceId) -> Result<()>;

    /// 断开设备连接
    async fn disconnect_device(&self, device_id: DeviceId) -> Result<()>;

    /// 订阅事件
    fn subscribe_events(&self) -> EventSubscriber;

    /// 获取统计信息
    async fn get_stats(&self) -> Result<SyncStats>;
}

/// 文件夹状态
#[derive(Debug, Clone)]
pub struct FolderState {
    pub folder_id: String,
    pub status: FolderStatus,
    pub last_scan: Option<chrono::DateTime<chrono::Utc>>,
    pub last_pull: Option<chrono::DateTime<chrono::Utc>>,
    pub local_files: usize,
    pub remote_files: usize,
    pub in_sync_files: usize,
    pub need_files: usize,
    pub errors: Vec<String>,
}

impl FolderState {
    pub fn new(folder_id: impl Into<String>) -> Self {
        Self {
            folder_id: folder_id.into(),
            status: FolderStatus::Idle,
            last_scan: None,
            last_pull: None,
            local_files: 0,
            remote_files: 0,
            in_sync_files: 0,
            need_files: 0,
            errors: Vec::new(),
        }
    }

    /// 计算完成度百分比
    pub fn completion(&self) -> u8 {
        if self.local_files == 0 {
            return 100;
        }
        let synced = self.local_files.saturating_sub(self.need_files);
        ((synced as f64 / self.local_files as f64) * 100.0) as u8
    }
}

/// 同步统计
#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    pub folders: HashMap<String, FolderStats>,
    pub total_files: usize,
    pub total_bytes: u64,
    pub uptime_secs: u64,
}

/// 文件夹统计
#[derive(Debug, Clone, Default)]
pub struct FolderStats {
    pub files: usize,
    pub directories: usize,
    pub symlinks: usize,
    pub total_bytes: u64,
    pub deleted: usize,
}

/// 同步任务句柄
pub struct SyncTaskHandle {
    pub folder_id: String,
    pub task_type: SyncTaskType,
    pub abort_handle: tokio::task::AbortHandle,
}

/// 同步任务类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTaskType {
    Scan,
    Pull,
    Push,
}

/// 文件夹配置与状态
#[derive(Debug, Clone)]
pub struct FolderModel {
    pub config: Folder,
    pub state: FolderState,
    pub local_sequence: u64,
    pub remote_devices: Vec<DeviceId>,
}

impl FolderModel {
    pub fn new(config: Folder) -> Self {
        let folder_id = config.id.clone();
        Self {
            config,
            state: FolderState::new(folder_id),
            local_sequence: 0,
            remote_devices: Vec::new(),
        }
    }

    /// 检查是否应该扫描
    pub fn should_scan(&self) -> bool {
        if self.config.paused {
            return false;
        }
        
        match self.state.status {
            FolderStatus::Idle | FolderStatus::SyncWaiting => true,
            _ => false,
        }
    }

    /// 检查是否应该拉取
    pub fn should_pull(&self) -> bool {
        if self.config.paused {
            return false;
        }
        
        // 只有发送接收和仅接收文件夹可以拉取
        if !self.config.folder_type.can_sync() {
            return false;
        }

        match self.state.status {
            FolderStatus::Idle | FolderStatus::ScanWaiting | FolderStatus::SyncWaiting => true,
            _ => false,
        }
    }
}
