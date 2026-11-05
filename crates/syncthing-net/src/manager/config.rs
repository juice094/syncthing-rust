use std::net::SocketAddr;
use std::time::Duration;

use syncthing_core::RetryConfig;

use crate::tcp_transport::DEFAULT_TCP_PORT;

/// Connection manager configuration.
#[derive(Debug, Clone)]
pub struct ConnectionManagerConfig {
    /// Listen address.
    pub listen_addr: SocketAddr,
    /// Retry configuration.
    pub retry_config: RetryConfig,
    /// Heartbeat interval.
    pub heartbeat_interval: Duration,
    /// Idle connection timeout.
    pub connection_timeout: Duration,
    /// Maximum concurrent connections (enforced at accept time).
    pub max_connections: usize,
}

impl Default for ConnectionManagerConfig {
    fn default() -> Self {
        Self {
            listen_addr: ([0, 0, 0, 0], DEFAULT_TCP_PORT).into(),
            retry_config: RetryConfig::default(),
            heartbeat_interval: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(120),
            max_connections: 1000,
        }
    }
}
