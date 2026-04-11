//! 错误处理模块
//! 
//! 基于Go版本的错误处理模式，提供统一的错误类型

use std::io;

/// Syncthing 错误类型
#[derive(Debug, thiserror::Error)]
pub enum SyncthingError {
    /// I/O 错误
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    
    /// TLS 错误
    #[error("TLS error: {0}")]
    Tls(String),
    
    /// Rustls 错误
    #[error("Rustls error: {0}")]
    Rustls(String),
    
    /// 连接错误
    #[error("Connection error: {message}")]
    Connection { message: String },
    
    /// 握手错误
    #[error("Handshake error: {message}")]
    Handshake { message: String },
    
    /// 协议错误
    #[error("Protocol error: {message}")]
    Protocol { message: String },
    
    /// 配置错误
    #[error("Configuration error: {message}")]
    Config { message: String },
    
    /// 设备ID错误
    #[error("Device ID error: {message}")]
    DeviceId { message: String },
    
    /// 消息序列化错误
    #[error("Message serialization error: {0}")]
    Serialization(String),
    
    /// 消息解析错误
    #[error("Message parse error: {0}")]
    Parse(String),
    
    /// 超时错误
    #[error("Timeout: {context}")]
    Timeout { context: String },
    
    /// 未找到设备
    #[error("Device not found: {device_id}")]
    DeviceNotFound { device_id: String },
    
    /// 连接已关闭
    #[error("Connection closed")]
    ConnectionClosed,
    
    /// 存储错误
    #[error("Storage error: {0}")]
    Storage(String),
    
    /// 内部错误
    #[error("Internal error: {0}")]
    Internal(String),
    
    /// 网络错误
    #[error("Network error: {0}")]
    Network(String),
    
    /// 其他错误
    #[error("{0}")]
    Other(String),
}

impl SyncthingError {
    /// 创建连接错误
    pub fn connection<S: Into<String>>(message: S) -> Self {
        Self::Connection { message: message.into() }
    }
    
    /// 创建握手错误
    pub fn handshake<S: Into<String>>(message: S) -> Self {
        Self::Handshake { message: message.into() }
    }
    
    /// 创建协议错误
    pub fn protocol<S: Into<String>>(message: S) -> Self {
        Self::Protocol { message: message.into() }
    }
    
    /// 创建配置错误
    pub fn config<S: Into<String>>(message: S) -> Self {
        Self::Config { message: message.into() }
    }
    
    /// 创建设备ID错误
    pub fn device_id<S: Into<String>>(message: S) -> Self {
        Self::DeviceId { message: message.into() }
    }
    
    /// 创建 I/O 错误（从字符串描述）
    pub fn io<S: Into<String>>(message: S) -> Self {
        Self::Io(io::Error::new(io::ErrorKind::Other, message.into()))
    }
    
    /// 创建存储错误
    pub fn storage<S: Into<String>>(message: S) -> Self {
        Self::Storage(message.into())
    }
    
    /// 创建内部错误
    pub fn internal<S: Into<String>>(message: S) -> Self {
        Self::Internal(message.into())
    }
    
    /// 创建网络错误
    pub fn network<S: Into<String>>(message: S) -> Self {
        Self::Network(message.into())
    }
    
    /// 创建超时错误
    pub fn timeout<S: Into<String>>(context: S) -> Self {
        Self::Timeout { context: context.into() }
    }
    
    /// 检查是否为暂时性错误（适合重试）
    pub fn is_temporary(&self) -> bool {
        matches!(
            self,
            SyncthingError::Io(_)
                | SyncthingError::Connection { .. }
                | SyncthingError::Timeout { .. }
                | SyncthingError::ConnectionClosed
        )
    }
    
    /// 检查是否为致命错误（不应重试）
    pub fn is_fatal(&self) -> bool {
        !self.is_temporary()
    }
}

/// 结果类型别名
pub type Result<T> = std::result::Result<T, SyncthingError>;

/// 添加上下文到错误
trait Context<T> {
    fn context(self, msg: &str) -> Result<T>;
}

impl<T> Context<T> for io::Result<T> {
    fn context(self, _msg: &str) -> Result<T> {
        self.map_err(|e| SyncthingError::Io(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_kinds() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: SyncthingError = io_err.into();
        assert!(err.is_temporary());
        
        let protocol_err = SyncthingError::protocol("invalid magic");
        assert!(protocol_err.is_fatal());
    }
    
    #[test]
    fn test_error_display() {
        let err = SyncthingError::connection("refused");
        assert_eq!(err.to_string(), "Connection error: refused");
    }
}
