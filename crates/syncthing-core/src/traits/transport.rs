//! Transport layer trait definitions.
//!
//! `TransportType`, `PathQuality`, `ReliablePipe`, `BoxedPipe`,
//! `Transport`, and `TransportListener`.
//!
//! These traits define the contracts between modules; implementations must
//! match them exactly.

use async_trait::async_trait;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::DeviceId;

/// Transport type identifier for a connection path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportType {
    /// Plain TCP or TCP with TLS
    Tcp,
    /// QUIC
    Quic,
    /// Relay / DERP fallback
    Relay,
    /// WebSocket (for firewall traversal)
    WebSocket,
    /// Proxied connection (SOCKS5 / HTTP)
    Proxy,
    /// In-memory pipe (for testing)
    Memory,
    /// Other / unknown
    Other,
}

/// Quality metrics for a network path.
#[derive(Debug, Clone)]
pub struct PathQuality {
    /// Smoothed round-trip time
    pub rtt: Duration,
    /// Packet loss ratio [0.0, 1.0]
    pub packet_loss: f64,
    /// Estimated bandwidth in bits per second, if known
    pub estimated_bps: Option<u64>,
    /// When this measurement was last updated
    pub last_updated: Instant,
}

impl Default for PathQuality {
    fn default() -> Self {
        Self {
            rtt: Duration::from_secs(1),
            packet_loss: 0.0,
            estimated_bps: None,
            last_updated: Instant::now(),
        }
    }
}

/// A generic reliable byte pipe abstracting TCP, QUIC, relay, or in-memory transports.
///
/// Implementors must provide [`tokio::io::AsyncRead`] and [`tokio::io::AsyncWrite`]
/// implementations so that BEP codec can operate on the pipe without knowing the
/// underlying transport.
pub trait ReliablePipe: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Sync + Unpin {
    /// Local endpoint address, if meaningful for the transport.
    fn local_addr(&self) -> Option<SocketAddr>;

    /// Peer endpoint address, if meaningful for the transport.
    fn peer_addr(&self) -> Option<SocketAddr>;

    /// Current quality estimate for this path.
    fn path_quality(&self) -> PathQuality {
        PathQuality::default()
    }

    /// Transport type discriminator.
    fn transport_type(&self) -> TransportType;
}

/// Type alias for boxed reliable pipes used by BEP connections.
pub type BoxedPipe = Box<dyn ReliablePipe>;

/// Transport layer abstraction.
///
/// Establishes raw byte pipes (TCP/QUIC/WebSocket/proxy); it does not deal
/// with TLS, authentication, or the BEP protocol. Concrete implementations
/// (`TcpTransport` / `WebSocketTransport` / `QuicTransport`) live in the
/// syncthing-net crate.
#[async_trait]
pub trait Transport: Send + Sync + std::fmt::Debug {
    /// Returns the transport scheme name (e.g. "tcp", "quic", "websocket", "proxy", "derp").
    fn scheme(&self) -> &'static str;

    /// Starts listening on the given address.
    ///
    /// The returned `TransportListener` accepts incoming connections.
    async fn bind(&self, addr: SocketAddr) -> Result<Box<dyn TransportListener>>;

    /// Dials the given address.
    ///
    /// The returned `BoxedPipe` can be used directly for BEP or TLS handshakes.
    async fn dial(&self, addr: SocketAddr) -> Result<BoxedPipe>;

    /// Dials a specific device (with device ID context).
    ///
    /// Used by relay transports such as DERP, which need the target device ID
    /// for routing. The default implementation falls back to plain `dial(addr)`.
    async fn dial_device(&self, addr: SocketAddr, _device_id: &DeviceId) -> Result<BoxedPipe> {
        self.dial(addr).await
    }
}

/// Listener abstraction.
///
/// Paired with [`Transport`]; accepts incoming raw connections.
#[async_trait]
pub trait TransportListener: Send + Sync + std::fmt::Debug {
    /// Accepts the next incoming connection.
    ///
    /// Returns `(pipe, peer address)`. Errors if the listener is closed.
    async fn accept(&self) -> Result<(BoxedPipe, SocketAddr)>;

    /// Returns the local listen address.
    fn local_addr(&self) -> Result<SocketAddr>;
}
