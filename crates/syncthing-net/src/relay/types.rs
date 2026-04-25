//! Relay 协议类型定义与错误类型

/// Relay 客户端错误
#[derive(Debug)]
pub enum RelayError {
    /// 协议级错误（编解码、非法消息等）
    Protocol(String),
    /// 连接已关闭
    ConnectionClosed,
    /// 服务器拒绝
    Rejected(String),
    /// 中继已满
    RelayFull,
    /// 对端设备未在中继上注册
    PeerNotFound,
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(s) => write!(f, "relay protocol error: {}", s),
            Self::ConnectionClosed => write!(f, "relay connection closed"),
            Self::Rejected(s) => write!(f, "relay rejected: {}", s),
            Self::RelayFull => write!(f, "relay full"),
            Self::PeerNotFound => write!(f, "peer not found on relay"),
        }
    }
}

impl std::error::Error for RelayError {}

impl From<RelayError> for syncthing_core::SyncthingError {
    fn from(e: RelayError) -> Self {
        syncthing_core::SyncthingError::network(e.to_string())
    }
}

/// Relay 操作结果
pub type Result<T> = std::result::Result<T, RelayError>;
