//! Syncthing 网络层库 (简化版)
//!
//! 提供TCP连接、TLS握手、BEP协议实现的简化接口
//! 以及 NAT 穿透功能（STUN/UPnP）

pub mod connection;
#[doc(hidden)]
pub mod derp;
pub mod dialer;
pub mod discovery;
#[doc(hidden)]
pub mod handshaker;
pub mod identity;
pub mod manager;
pub mod metrics;
pub mod netmon;
pub mod portmapper;
pub mod protocol;
#[doc(hidden)]
pub mod relay;
pub mod session;
pub mod stun;
pub mod tcp_transport;
pub mod tls;
#[doc(hidden)]
pub mod transport;
pub mod upnp;

/// Stable public API exports.
pub use connection::{BepConnection, ConnectionEvent, TcpBiStream};
pub use dialer::{
    AddressScore, AddressTypePreference, DialConnector, ParallelDialer, TcpBepConnector,
};
pub use discovery::{AddressInfo, AddressSource, DiscoveryConfig, DiscoveryManager};
pub use discovery::{DiscoveryEvent, DiscoverySource, LocalDiscovery};
pub use discovery::{GlobalDiscovery, ANNOUNCE_INTERVAL, DEFAULT_DISCOVERY_SERVER, RETRY_INTERVAL};
pub use handshaker::BepHandshaker;
pub use identity::TlsIdentity;
pub use manager::{
    ConnectionManager, ConnectionManagerConfig, ConnectionManagerHandle, ManagerStats,
};
pub use metrics::{global, MetricRecord, MetricsCollector};
pub use netmon::{NetChangeEvent, NetMonitor};
pub use portmapper::{Mapping, PortMapper};
pub use protocol::{HelloMessage, MessageType, BEP_MAGIC};
pub use session::{BepSession, BepSessionEvent, BepSessionHandler, BepSessionMetrics};
pub use stun::{query, StunClient, StunRefresher, DEFAULT_STUN_SERVERS};
pub use tcp_transport::{TcpDialer, TcpTransport, DEFAULT_TCP_PORT};
pub use tls::{accept_tls, connect_tls, generate_certificate, SyncthingTlsConfig};
pub use upnp::{discover_upnp, UpnpClient, UpnpMappingManager, DEFAULT_MAPPING_DURATION};

/// TLS 相关常量
pub mod tls_constants {
    pub use super::tls::{CERT_FILE_NAME, KEY_FILE_NAME};
}

/// 版本信息
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
#[ctor::ctor]
fn init_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
