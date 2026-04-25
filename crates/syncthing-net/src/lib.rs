//! Syncthing 网络层库 (简化版)
//!
//! 提供TCP连接、TLS握手、BEP协议实现的简化接口
//! 以及 NAT 穿透功能（STUN/UPnP）

pub mod connection;
pub mod handshaker;
pub mod manager;
pub mod netmon;
pub mod session;
pub mod tcp_transport;
pub mod tls;
pub mod protocol;
pub mod stun;
pub mod upnp;
pub mod discovery;
pub mod portmapper;
pub mod dialer;
pub mod metrics;
pub mod identity;
pub mod transport;
pub mod derp;
pub mod relay;

pub use connection::{BepConnection, ConnectionEvent, TcpBiStream};
pub use session::{BepSession, BepSessionEvent, BepSessionHandler, BepSessionMetrics};
pub use protocol::{HelloMessage, MessageType, BEP_MAGIC};
pub use manager::{ConnectionManager, ConnectionManagerConfig, ConnectionManagerHandle, ManagerStats};
pub use netmon::{NetMonitor, NetChangeEvent};
pub use tcp_transport::{TcpTransport, TcpDialer, DEFAULT_TCP_PORT};
pub use dialer::{ParallelDialer, AddressScore, AddressTypePreference, DialConnector, TcpBepConnector};
pub use tls::{SyncthingTlsConfig, accept_tls, connect_tls, generate_certificate};
pub use stun::{query, StunClient, StunRefresher, DEFAULT_STUN_SERVERS};
pub use upnp::{UpnpClient, UpnpMappingManager, discover_upnp, DEFAULT_MAPPING_DURATION};
pub use discovery::{DiscoveryManager, DiscoveryConfig, AddressInfo, AddressSource};
pub use discovery::{LocalDiscovery, DiscoveryEvent, DiscoverySource};
pub use discovery::{GlobalDiscovery, DEFAULT_DISCOVERY_SERVER, ANNOUNCE_INTERVAL, RETRY_INTERVAL};
pub use portmapper::{PortMapper, Mapping};

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
