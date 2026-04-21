//! Transport → BEP 适配层
//!
//! 将原始 `Transport` 字节管道提升为完整的 BEP 连接。
//! 负责在 `BoxedPipe` 之上执行 TLS 握手 + BEP Hello，
//! 产出可供 `ConnectionManager` 直接使用的 `Arc<BepConnection>`。

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tracing::{debug, error, info, warn};

use syncthing_core::{
    BoxedPipe, ConnectionType, DeviceId, PathQuality, ReliablePipe, Result as CoreResult,
    SyncthingError, TransportType,
};

use crate::connection::BepConnection;
use crate::dialer::DialConnector;
use crate::handshaker::BepHandshaker;
use crate::tls::SyncthingTlsConfig;
use syncthing_core::Transport;

/// 包装任意 TLS 流为 `ReliablePipe`。
///
/// 由于 orphan rule 限制，无法为 `tokio_rustls::TlsStream<BoxedPipe>` 直接 impl `ReliablePipe`，
/// 因此引入此轻量 wrapper。
pub struct TlsPipe<S> {
    stream: S,
    local_addr: Option<SocketAddr>,
    peer_addr: Option<SocketAddr>,
}

impl<S> TlsPipe<S> {
    pub fn new(stream: S, local_addr: Option<SocketAddr>, peer_addr: Option<SocketAddr>) -> Self {
        Self {
            stream,
            local_addr,
            peer_addr,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for TlsPipe<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for TlsPipe<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + AsyncWrite + Send + Sync + Unpin> ReliablePipe for TlsPipe<S> {
    fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Tcp
    }

    fn path_quality(&self) -> PathQuality {
        PathQuality::default()
    }
}

/// 基于 `Transport` trait 的 `DialConnector` 实现。
///
/// 拨号流程：
/// 1. `transport.dial(addr)` → `BoxedPipe`
/// 2. `connect_tls_stream(pipe)` → `ClientTlsStream<BoxedPipe>`
/// 3. `BepHandshaker::client_handshake` → BEP Hello 完成
/// 4. 包装为 `BepConnection` 返回
pub struct TransportBepConnector {
    transport: Arc<dyn Transport>,
}

impl TransportBepConnector {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport }
    }
}

#[async_trait::async_trait]
impl DialConnector for TransportBepConnector {
    async fn connect(
        &self,
        addr: SocketAddr,
        device_id: DeviceId,
        _local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> CoreResult<Arc<BepConnection>> {
        debug!("TransportBepConnector dialing {} for device {}", addr, device_id);

        // 1. 原始传输层拨号
        let pipe = self
            .transport
            .dial(addr)
            .await
            .map_err(|e| SyncthingError::connection(format!("transport dial failed: {}", e)))?;

        let local_addr = pipe.local_addr();
        let peer_addr = pipe.peer_addr();

        // 2. TLS 客户端握手
        let (tls_stream, _remote_device_id) =
            crate::tls::connect_tls_stream(pipe, tls_config, Some(device_id)).await?;

        debug!("TLS handshake completed with {}", addr);

        // 3. BEP Hello 交换
        let mut tls_stream = tls_stream;
        let _remote_hello = BepHandshaker::client_handshake(&mut tls_stream, device_name).await?;

        // 4. 创建 BEP 连接
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let tls_pipe = TlsPipe::new(tls_stream, local_addr, peer_addr);
        let conn = BepConnection::new(Box::new(tls_pipe), ConnectionType::Outgoing, event_tx).await?;

        conn.set_device_id(device_id);
        conn.set_state(syncthing_core::ConnectionState::ProtocolHandshakeComplete);

        info!("BEP connection established to {} (device: {})", addr, device_id);
        Ok(conn)
    }
}

/// 基于 `TransportListener` 的通用 BEP 监听循环。
///
/// 监听流程：
/// 1. `transport.listen(bind_addr)` → `Box<dyn TransportListener>`
/// 2. `listener.accept()` → `BoxedPipe`
/// 3. `accept_tls_stream(pipe)` → `ServerTlsStream<BoxedPipe>`
/// 4. `BepHandshaker::server_handshake` → BEP Hello 完成
/// 5. 注册到 `ConnectionManagerHandle`
pub struct BepTransportListener;

impl BepTransportListener {
    /// 启动监听循环。
    ///
    /// 该函数会 spawn 一个后台任务，持续 accept 传入连接。
    pub async fn start(
        transport: Arc<dyn Transport>,
        bind_addr: &str,
        manager: crate::manager::ConnectionManagerHandle,
        _local_device_id: DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> CoreResult<SocketAddr> {
        let bind_addr: SocketAddr = bind_addr.parse().map_err(|e| {
            SyncthingError::config(format!("invalid bind address: {}", e))
        })?;
        let listener = transport.bind(bind_addr).await?;
        let listen_addr = listener.local_addr()?;

        info!(
            "BepTransportListener starting on {} (scheme: {})",
            listen_addr,
            transport.scheme()
        );

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((pipe, _peer_addr)) => {
                        let manager = manager.clone();
                        let device_name = device_name.clone();
                        let tls_config = Arc::clone(&tls_config);

                        tokio::spawn(async move {
                            if let Err(e) = Self::handle_incoming(pipe, manager, &device_name, tls_config).await {
                                warn!("Failed to handle incoming BEP connection: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Transport accept error: {}", e);
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        });

        Ok(listen_addr)
    }

    async fn handle_incoming(
        pipe: BoxedPipe,
        manager: crate::manager::ConnectionManagerHandle,
        device_name: &str,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> CoreResult<()> {
        let local_addr = pipe.local_addr();
        let peer_addr = pipe.peer_addr();

        // 服务端 TLS 握手
        let (tls_stream, device_id) = crate::tls::accept_tls_stream(pipe, &tls_config).await?;
        debug!("Server TLS handshake completed: peer_device_id={}", device_id);

        // BEP Hello 交换
        let mut tls_stream = tls_stream;
        let _remote_hello = BepHandshaker::server_handshake(&mut tls_stream, device_name).await?;

        // 创建 BEP 连接
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let tls_pipe = TlsPipe::new(tls_stream, local_addr, peer_addr);
        let conn = BepConnection::new(Box::new(tls_pipe), ConnectionType::Incoming, event_tx).await?;

        conn.set_device_id(device_id);
        manager.register_connection(device_id, conn).await?;

        info!("Incoming BEP connection registered for device {}", device_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tls_pipe_boxed_roundtrip() {
        // 验证 TlsPipe 能被正确装箱为 BoxedPipe
        // 使用内存管道作为底层
        let (client, server) = syncthing_test_utils::memory_pipe_pair(1024);

        // TlsPipe 包装 client 端
        let _pipe: BoxedPipe = Box::new(TlsPipe::new(client, None, None));

        // 只验证类型系统通过即可
        let _ = _pipe;
        let _ = server;
    }
}
