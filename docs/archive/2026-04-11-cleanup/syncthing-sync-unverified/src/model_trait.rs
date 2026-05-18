//! Module: syncthing-sync
//! Worker: Agent-Sync
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Model trait 定义 - BEP 协议消息处理接口
//!
//! 该模块定义了 Syncthing 核心同步逻辑的 Model trait，对应 Go 实现中的 Model 接口。
//! Model 负责处理来自远程设备的 BEP 协议消息。

use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;


use syncthing_core::types::{DeviceId, FileInfo, FolderId};
use syncthing_core::Result;

/// 完整索引消息
#[derive(Debug, Clone)]
pub struct Index {
    /// 文件夹 ID
    pub folder: FolderId,
    /// 文件列表
    pub files: Vec<FileInfo>,
}

/// 索引更新消息（增量）
#[derive(Debug, Clone)]
pub struct IndexUpdate {
    /// 文件夹 ID
    pub folder: FolderId,
    /// 更新的文件列表
    pub files: Vec<FileInfo>,
}

/// 块请求消息
#[derive(Debug, Clone)]
pub struct Request {
    /// 文件夹 ID
    pub folder: FolderId,
    /// 文件名
    pub name: String,
    /// 块哈希
    pub hash: syncthing_core::types::BlockHash,
    /// 文件内偏移量
    pub offset: u64,
    /// 请求大小
    pub size: usize,
}

/// 集群配置消息
#[derive(Debug, Clone)]
pub struct ClusterConfig {
    /// 设备列表
    pub devices: Vec<DeviceInfo>,
    /// 文件夹列表
    pub folders: Vec<FolderInfo>,
}

/// 设备信息
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// 设备 ID
    pub id: DeviceId,
    /// 设备名称
    pub name: String,
    /// 设备地址
    pub addresses: Vec<String>,
}

/// 文件夹信息
#[derive(Debug, Clone)]
pub struct FolderInfo {
    /// 文件夹 ID
    pub id: FolderId,
    /// 文件夹标签
    pub label: String,
    /// 共享的设备 ID 列表
    pub device_ids: Vec<DeviceId>,
}

/// 下载进度更新
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    /// 文件夹 ID
    pub folder: FolderId,
    /// 文件名
    pub file: String,
    /// 总大小
    pub total: u64,
    /// 已完成大小
    pub done: u64,
}

/// Model trait - BEP 协议消息处理接口
///
/// 对应 Go 实现中的 Model 接口 (lib/protocol/protocol.go:78-91)
/// 负责处理来自远程设备的各种 BEP 协议消息。
#[async_trait]
pub trait Model: Send + Sync + fmt::Debug {
    /// 收到完整索引
    ///
    /// 当远程设备发送完整索引时调用。通常在初始连接建立后接收。
    ///
    /// # 参数
    /// * `device` - 远程设备 ID
    /// * `folder` - 文件夹名称
    /// * `files` - 文件列表
    async fn index(&self, device: DeviceId, folder: &str, files: Vec<FileInfo>) -> Result<()>;

    /// 收到索引更新
    ///
    /// 当远程设备发送索引更新（增量）时调用。通常在文件发生变化时接收。
    ///
    /// # 参数
    /// * `device` - 远程设备 ID
    /// * `folder` - 文件夹名称
    /// * `files` - 更新的文件列表
    async fn index_update(&self, device: DeviceId, folder: &str, files: Vec<FileInfo>) -> Result<()>;

    /// 收到块请求
    ///
    /// 当远程设备请求块数据时调用。需要返回请求的数据。
    ///
    /// # 参数
    /// * `folder` - 文件夹名称
    /// * `name` - 文件名
    /// * `offset` - 文件内偏移量
    /// * `size` - 请求大小
    /// * `hash` - 块哈希
    ///
    /// # 返回
    /// 请求的数据块
    async fn request(
        &self,
        folder: &str,
        name: &str,
        offset: i64,
        size: i32,
        hash: &[u8],
    ) -> Result<Vec<u8>>;

    /// 收到集群配置
    ///
    /// 当远程设备发送集群配置时调用。包含设备和文件夹的共享信息。
    ///
    /// # 参数
    /// * `device` - 远程设备 ID
    /// * `config` - 集群配置
    async fn cluster_config(&self, device: DeviceId, config: &ClusterConfig) -> Result<()>;

    /// 连接关闭
    ///
    /// 当连接关闭时调用。可以用于清理资源。
    ///
    /// # 参数
    /// * `device` - 远程设备 ID
    /// * `err` - 关闭原因（None 表示正常关闭）
    async fn closed(&self, device: DeviceId, err: Option<&syncthing_core::SyncthingError>);
}

/// Model 扩展 trait
///
/// 提供额外的 Model 相关功能
#[async_trait]
pub trait ModelExt: Model {
    /// 获取文件夹状态
    async fn folder_state(&self, folder: &FolderId) -> Result<FolderState>;

    /// 触发拉取操作
    async fn trigger_pull(&self, folder: &FolderId) -> Result<()>;

    /// 暂停文件夹同步
    async fn pause_folder(&self, folder: &FolderId) -> Result<()>;

    /// 恢复文件夹同步
    async fn resume_folder(&self, folder: &FolderId) -> Result<()>;
}

/// 文件夹状态机
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderState {
    /// 空闲状态，已同步
    Idle,
    /// 正在扫描本地文件
    Scanning,
    /// 正在同步（拉取中）
    Syncing {
        /// 总任务数
        total: usize,
        /// 已完成任务数
        done: usize,
    },
    /// 错误状态
    Error {
        /// 错误信息
        message: String
    },
    /// 已暂停
    Paused,
}

impl fmt::Display for FolderState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FolderState::Idle => write!(f, "idle"),
            FolderState::Scanning => write!(f, "scanning"),
            FolderState::Syncing { total, done } => {
                let progress = if *total > 0 {
                    (*done as f64 / *total as f64) * 100.0
                } else {
                    0.0
                };
                write!(f, "syncing ({:.1}%)", progress)
            }
            FolderState::Error { message } => write!(f, "error: {}", message),
            FolderState::Paused => write!(f, "paused"),
        }
    }
}

impl FolderState {
    /// 检查是否处于同步状态
    pub fn is_syncing(&self) -> bool {
        matches!(self, FolderState::Syncing { .. })
    }

    /// 检查是否处于扫描状态
    pub fn is_scanning(&self) -> bool {
        matches!(self, FolderState::Scanning)
    }

    /// 检查是否处于活动状态
    pub fn is_active(&self) -> bool {
        matches!(self, FolderState::Scanning | FolderState::Syncing { .. })
    }

    /// 检查是否可以启动同步
    pub fn can_start_sync(&self) -> bool {
        matches!(self, FolderState::Idle | FolderState::Error { .. } | FolderState::Paused)
    }

    /// 获取进度（如果有）
    pub fn progress(&self) -> Option<f64> {
        match self {
            FolderState::Syncing { total, done } => {
                if *total > 0 {
                    Some(*done as f64 / *total as f64)
                } else {
                    Some(0.0)
                }
            }
            _ => None,
        }
    }
}

/// 远程设备状态
#[derive(Debug, Clone)]
pub struct RemoteDeviceState {
    /// 设备索引映射：路径 -> FileInfo
    pub index: HashMap<String, FileInfo>,
    /// 下载进度
    pub download_progress: HashMap<String, DownloadProgress>,
    /// 最后活动时间
    pub last_active: std::time::Instant,
}

impl RemoteDeviceState {
    /// 创建新的远程设备状态
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
            download_progress: HashMap::new(),
            last_active: std::time::Instant::now(),
        }
    }

    /// 更新文件索引
    pub fn update_files(&mut self, files: Vec<FileInfo>) {
        for file in files {
            self.index.insert(file.name.clone(), file);
        }
        self.last_active = std::time::Instant::now();
    }

    /// 获取文件信息
    pub fn get_file(&self, name: &str) -> Option<&FileInfo> {
        self.index.get(name)
    }

    /// 更新下载进度
    pub fn update_progress(&mut self, progress: DownloadProgress) {
        self.download_progress
            .insert(progress.file.clone(), progress);
        self.last_active = std::time::Instant::now();
    }
}

impl Default for RemoteDeviceState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_folder_state_display() {
        assert_eq!(FolderState::Idle.to_string(), "idle");
        assert_eq!(FolderState::Scanning.to_string(), "scanning");
        assert!(FolderState::Syncing { total: 10, done: 5 }
            .to_string()
            .contains("syncing"));
        assert!(FolderState::Error {
            message: "test".to_string()
        }
        .to_string()
        .contains("error"));
        assert_eq!(FolderState::Paused.to_string(), "paused");
    }

    #[test]
    fn test_folder_state_is_syncing() {
        assert!(!FolderState::Idle.is_syncing());
        assert!(!FolderState::Scanning.is_syncing());
        assert!(FolderState::Syncing { total: 10, done: 5 }.is_syncing());
        assert!(!FolderState::Error {
            message: "test".to_string()
        }
        .is_syncing());
        assert!(!FolderState::Paused.is_syncing());
    }

    #[test]
    fn test_folder_state_is_scanning() {
        assert!(!FolderState::Idle.is_scanning());
        assert!(FolderState::Scanning.is_scanning());
        assert!(!FolderState::Syncing { total: 10, done: 5 }.is_scanning());
        assert!(!FolderState::Paused.is_scanning());
    }

    #[test]
    fn test_folder_state_is_active() {
        assert!(!FolderState::Idle.is_active());
        assert!(FolderState::Scanning.is_active());
        assert!(FolderState::Syncing { total: 10, done: 5 }.is_active());
        assert!(!FolderState::Error {
            message: "test".to_string()
        }
        .is_active());
        assert!(!FolderState::Paused.is_active());
    }

    #[test]
    fn test_folder_state_can_start_sync() {
        assert!(FolderState::Idle.can_start_sync());
        assert!(!FolderState::Scanning.can_start_sync());
        assert!(!FolderState::Syncing { total: 10, done: 5 }.can_start_sync());
        assert!(FolderState::Error {
            message: "test".to_string()
        }
        .can_start_sync());
        assert!(FolderState::Paused.can_start_sync());
    }

    #[test]
    fn test_folder_state_progress() {
        assert_eq!(FolderState::Idle.progress(), None);
        assert_eq!(FolderState::Scanning.progress(), None);
        assert_eq!(
            FolderState::Syncing { total: 10, done: 5 }.progress(),
            Some(0.5)
        );
        assert_eq!(
            FolderState::Syncing { total: 0, done: 0 }.progress(),
            Some(0.0)
        );
    }

    #[test]
    fn test_remote_device_state() {
        let mut state = RemoteDeviceState::new();
        assert!(state.index.is_empty());

        let file = FileInfo::new("test.txt");
        state.update_files(vec![file.clone()]);
        assert_eq!(state.index.len(), 1);
        assert!(state.get_file("test.txt").is_some());

        let progress = DownloadProgress {
            folder: FolderId::new("test"),
            file: "test.txt".to_string(),
            total: 100,
            done: 50,
        };
        state.update_progress(progress);
        assert!(state.download_progress.contains_key("test.txt"));
    }
}
