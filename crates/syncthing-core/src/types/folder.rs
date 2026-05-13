//! 文件夹（Folder）相关类型
//!
//! 2026-05-13 (T1.2)：从 `types/mod.rs` 抽离，避免单文件超过 600 行。
//! 包含 `FolderType` / `FolderStatus` / `Compression` / `Folder` / `FolderId` / `FolderSummary`。

use serde::{Deserialize, Serialize};
use std::fmt;

use super::VersioningConfig;
use crate::DeviceId;

/// 文件夹类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderType {
    /// 发送接收（双向同步）
    SendReceive,
    /// 仅发送
    SendOnly,
    /// 仅接收
    ReceiveOnly,
    /// 接收加密
    ReceiveEncrypted,
}

impl FolderType {
    /// 是否可以发送变更
    pub fn can_send(&self) -> bool {
        matches!(self, FolderType::SendReceive | FolderType::SendOnly)
    }

    /// 是否可以接收变更
    pub fn can_sync(&self) -> bool {
        matches!(
            self,
            FolderType::SendReceive | FolderType::ReceiveOnly | FolderType::ReceiveEncrypted
        )
    }
}

/// 文件夹状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderStatus {
    /// 空闲
    Idle,
    /// 等待扫描
    ScanWaiting,
    /// 正在扫描
    Scanning,
    /// 等待同步
    SyncWaiting,
    /// 正在同步（拉取）
    Pulling,
    /// 正在同步（推送）
    Pushing,
    /// 同步完成
    Synced,
    /// 暂停
    Paused,
    /// 错误
    Error,
}

/// 压缩模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Compression {
    #[default]
    Metadata,
    Always,
    Never,
}

/// 文件夹配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    /// 文件夹ID
    pub id: String,
    /// 文件夹路径
    pub path: String,
    /// 文件夹标签（可选）
    pub label: Option<String>,
    /// 文件夹类型
    pub folder_type: FolderType,
    /// 是否暂停
    pub paused: bool,
    /// 重新扫描间隔（秒）
    pub rescan_interval_secs: i32,
    /// 设备列表（哪些设备共享此文件夹）
    pub devices: Vec<DeviceId>,
    /// 忽略模式
    pub ignore_patterns: Vec<String>,
    /// 版本控制配置
    pub versioning: Option<VersioningConfig>,
}

impl Folder {
    /// 创建新的文件夹配置
    pub fn new(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            label: None,
            folder_type: FolderType::SendReceive,
            paused: false,
            rescan_interval_secs: 3600, // 默认1小时
            devices: Vec::new(),
            ignore_patterns: Vec::new(),
            versioning: None,
        }
    }
}

/// Folder identifier
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId(String);

impl FolderId {
    /// Create from string
    pub fn new<S: Into<String>>(id: S) -> Self {
        Self(id.into())
    }

    /// Get as string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FolderId({})", self.0)
    }
}

impl fmt::Display for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Folder sync summary
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderSummary {
    /// Total files
    pub files: u64,
    /// Total directories
    pub directories: u64,
    /// Total symlinks
    pub symlinks: u64,
    /// Total bytes
    pub bytes: u64,
    /// Files needing sync
    pub need_files: u64,
    /// Directories needing sync
    pub need_directories: u64,
    /// Bytes needing sync
    pub need_bytes: u64,
    /// Pull errors
    pub pull_errors: u32,
}

impl FolderSummary {
    /// Check if folder is in sync
    pub fn is_synced(&self) -> bool {
        self.need_files == 0 && self.need_directories == 0 && self.need_bytes == 0
    }

    /// Calculate sync percentage
    pub fn sync_percent(&self) -> f64 {
        if self.bytes == 0 {
            return 100.0;
        }
        let synced = self.bytes - self.need_bytes;
        (synced as f64 / self.bytes as f64) * 100.0
    }
}
