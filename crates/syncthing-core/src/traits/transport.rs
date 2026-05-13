//! Transport layer trait definitions.
//!
//! 2026-05-13 (T1.5)：从 `traits.rs` 抽离传输层相关的 trait/struct/enum，
//! 包括 `TransportType`、`PathQuality`、`ReliablePipe`、`BoxedPipe`、
//! `Transport`、`TransportListener`。
//!
//! ⚠️ CRITICAL: Maintained by Master Agent
//! These traits define the contracts between modules. Worker Agents
//! must implement these traits exactly as specified.

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
/// 传输层抽象。
///
/// 负责建立原始字节管道（TCP/QUIC/WebSocket/代理），
/// 不涉及 TLS、身份验证或 BEP 协议。
///
/// 这是 Phase 2 "多传输安全网络层" 的核心抽象。
/// 具体实现（TcpTransport / WebSocketTransport / QuicTransport）位于 syncthing-net crate。
#[async_trait]
pub trait Transport: Send + Sync + std::fmt::Debug {
    /// 传输方案名称（如 "tcp", "quic", "websocket", "proxy", "derp"）
    fn scheme(&self) -> &'static str;

    /// 在给定地址开始监听。
    ///
    /// 返回的 `TransportListener` 负责接受传入连接。
    async fn bind(&self, addr: SocketAddr) -> Result<Box<dyn TransportListener>>;

    /// 向给定地址拨号。
    ///
    /// 返回的 `BoxedPipe` 可直接用于 BEP 协议或 TLS 握手。
    async fn dial(&self, addr: SocketAddr) -> Result<BoxedPipe>;

    /// 向指定设备拨号（带设备 ID 上下文）。
    ///
    /// 用于 DERP 等中继传输，需要将目标设备 ID 告知中继服务器进行路由。
    /// 默认实现回退到普通 `dial(addr)`。
    async fn dial_device(&self, addr: SocketAddr, _device_id: &DeviceId) -> Result<BoxedPipe> {
        self.dial(addr).await
    }
}

/// 监听器抽象。
///
/// 与 `Transport` 配对使用，负责接受传入的原始连接。
#[async_trait]
pub trait TransportListener: Send + Sync + std::fmt::Debug {
    /// 接受下一个传入连接。
    ///
    /// 返回 `(管道, 对端地址)`。如果监听器已关闭，返回错误。
    async fn accept(&self) -> Result<(BoxedPipe, SocketAddr)>;

    /// 获取本地监听地址。
    fn local_addr(&self) -> Result<SocketAddr>;
}
