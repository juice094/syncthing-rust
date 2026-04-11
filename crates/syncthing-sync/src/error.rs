//! 同步错误处理

use std::io;
use std::path::PathBuf;

/// 同步错误类型
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// I/O 错误
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// 核心错误
    #[error("Core error: {0}")]
    Core(#[from] syncthing_core::SyncthingError),

    /// 扫描错误
    #[error("Scan error in folder '{folder}': {message}")]
    Scan { folder: String, message: String },

    /// 拉取错误
    #[error("Pull error for '{path}': {message}")]
    Pull { path: String, message: String },

    /// 数据库错误
    #[error("Database error: {0}")]
    Database(String),

    /// 文件夹不存在
    #[error("Folder not found: {0}")]
    FolderNotFound(String),

    /// 文件不存在
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    /// 索引错误
    #[error("Index error: {0}")]
    Index(String),

    /// 冲突错误
    #[error("Conflict error for '{path}': {message}")]
    Conflict { path: String, message: String },

    /// 校验和错误
    #[error("Checksum mismatch for block at offset {offset}")]
    ChecksumMismatch { offset: i64 },

    /// 超时错误
    #[error("Operation timeout: {context}")]
    Timeout { context: String },

    /// 取消错误
    #[error("Operation cancelled")]
    Cancelled,

    /// 配置错误
    #[error("Configuration error: {0}")]
    Config(String),

    /// 其他错误
    #[error("{0}")]
    Other(String),
}

impl SyncError {
    /// 创建扫描错误
    pub fn scan<S: Into<String>>(folder: S, message: S) -> Self {
        Self::Scan {
            folder: folder.into(),
            message: message.into(),
        }
    }

    /// 创建拉取错误
    pub fn pull<S: Into<String>>(path: S, message: S) -> Self {
        Self::Pull {
            path: path.into(),
            message: message.into(),
        }
    }

    /// 创建数据库错误
    pub fn database<S: Into<String>>(message: S) -> Self {
        Self::Database(message.into())
    }

    /// 创建索引错误
    pub fn index<S: Into<String>>(message: S) -> Self {
        Self::Index(message.into())
    }

    /// 创建冲突错误
    pub fn conflict<S: Into<String>>(path: S, message: S) -> Self {
        Self::Conflict {
            path: path.into(),
            message: message.into(),
        }
    }

    /// 创建超时错误
    pub fn timeout<S: Into<String>>(context: S) -> Self {
        Self::Timeout {
            context: context.into(),
        }
    }

    /// 检查是否为暂时性错误
    pub fn is_temporary(&self) -> bool {
        matches!(
            self,
            SyncError::Io(_)
                | SyncError::Timeout { .. }
                | SyncError::Pull { .. }
        )
    }
}

/// 结果类型别名
pub type Result<T> = std::result::Result<T, SyncError>;
