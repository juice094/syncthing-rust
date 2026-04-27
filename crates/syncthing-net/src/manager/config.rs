use std::net::SocketAddr;
use std::time::Duration;

use syncthing_core::RetryConfig;

use crate::tcp_transport::DEFAULT_TCP_PORT;

/// 连接管理器配置
#[derive(Debug, Clone)]
pub struct ConnectionManagerConfig {
    /// 监听地址
    pub listen_addr: SocketAddr,
    /// 重试配置
    pub retry_config: RetryConfig,
    /// 心跳间隔
    pub heartbeat_interval: Duration,
    /// 连接超时
    pub connection_timeout: Duration,
    /// 最大并发连接数
    pub max_connections: usize,
}

impl Default for ConnectionManagerConfig {
    fn default() -> Self {
        Self {
            listen_addr: ([0, 0, 0, 0], DEFAULT_TCP_PORT).into(),
            retry_config: RetryConfig::default(),
            heartbeat_interval: Duration::from_secs(90),
            connection_timeout: Duration::from_secs(120),
            max_connections: 1000,
        }
    }
}
