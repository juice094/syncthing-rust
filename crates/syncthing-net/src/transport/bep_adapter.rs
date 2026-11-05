//! Transport -> BEP adapter layer.
//!
//! Promotes a raw `Transport` byte pipe into a full BEP connection:
//! performs the TLS handshake + BEP Hello on top of `BoxedPipe` and produces
//! an `Arc<BepConnection>` ready for `ConnectionManager`.

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

/// Wraps any TLS stream as a `ReliablePipe`.
///
/// The orphan rule prevents implementing `ReliablePipe` directly for
/// `tokio_rustls::TlsStream<BoxedPipe>`, hence this lightweight wrapper.
pub struct TlsPipe<S> {
    stream: S,
    local_addr: Option<SocketAddr>,
    peer_addr: Option<SocketAddr>,
}

impl<S> TlsPipe<S> {
    /// Creates a wrapper around the stream with the given addresses.
    pub fn new(stream: S, local_addr: Option<SocketAddr>, peer_addr: Option<SocketAddr>) -> Self {
        Self {
            stream,
            local_addr,
            peer_addr,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for TlsPipe<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for TlsPipe<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
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

/// `DialConnector` implementation on top of the `Transport` trait.
///
/// Dial flow:
/// 1. `transport.dial(addr)` -> `BoxedPipe`
/// 2. `connect_tls_stream(pipe)` -> `ClientTlsStream<BoxedPipe>`
/// 3. `BepHandshaker::client_handshake` -> BEP Hello complete
/// 4. Wrapped into a `BepConnection`
pub struct TransportBepConnector {
    transport: Arc<dyn Transport>,
}

impl TransportBepConnector {
    /// Creates a connector over the given transport.
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
        local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> CoreResult<Arc<BepConnection>> {
        debug!(
            "TransportBepConnector dialing {} for device {}",
            addr, device_id
        );

        crate::manager::handshake::pre_handshake_yield(local_device_id, device_id).await;

        // Prefer dial_device so relay transports (e.g. DERP) are supported.
        let pipe = self
            .transport
            .dial_device(addr, &device_id)
            .await
            .map_err(|e| SyncthingError::connection(format!("transport dial failed: {}", e)))?;

        let local_addr = pipe.local_addr();
        let peer_addr = pipe.peer_addr();

        let (tls_stream, _remote_device_id) =
            crate::tls::connect_tls_stream(pipe, tls_config, Some(device_id)).await?;

        debug!("TLS handshake completed with {}", addr);

        let mut tls_stream = tls_stream;
        let _remote_hello = BepHandshaker::client_handshake(&mut tls_stream, device_name).await?;

        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
        let tls_pipe = TlsPipe::new(tls_stream, local_addr, peer_addr);
        let conn =
            BepConnection::new(Box::new(tls_pipe), ConnectionType::Outgoing, event_tx).await?;

        conn.set_device_id(device_id);
        conn.set_state(syncthing_core::ConnectionState::ProtocolHandshakeComplete);

        info!(
            "BEP connection established to {} (device: {})",
            addr, device_id
        );
        Ok(conn)
    }
}

/// Generic BEP listen loop on top of `TransportListener`.
///
/// Listen flow:
/// 1. `transport.listen(bind_addr)` -> `Box<dyn TransportListener>`
/// 2. `listener.accept()` -> `BoxedPipe`
/// 3. `accept_tls_stream(pipe)` -> `ServerTlsStream<BoxedPipe>`
/// 4. `BepHandshaker::server_handshake` -> BEP Hello complete
/// 5. Register with `ConnectionManagerHandle`
pub struct BepTransportListener;

impl BepTransportListener {
    /// Starts the listen loop.
    ///
    /// Spawns a background task that keeps accepting incoming connections.
    ///
    /// # Errors
    /// Returns an error if the bind address is invalid or the transport fails
    /// to bind.
    pub async fn start(
        transport: Arc<dyn Transport>,
        bind_addr: &str,
        manager: crate::manager::ConnectionManagerHandle,
        _local_device_id: DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> CoreResult<SocketAddr> {
        let bind_addr: SocketAddr = bind_addr
            .parse()
            .map_err(|e| SyncthingError::config(format!("invalid bind address: {}", e)))?;
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
                            if let Err(e) =
                                Self::handle_incoming(pipe, manager, &device_name, tls_config).await
                            {
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

        let (tls_stream, device_id) = crate::tls::accept_tls_stream(pipe, &tls_config).await?;
        debug!(
            "Server TLS handshake completed: peer_device_id={}",
            device_id
        );

        let _handshake_guard = manager
            .begin_handshake(device_id, ConnectionType::Incoming)
            .map_err(|e| {
                warn!(
                    "Incoming handshake race lost for {} before BEP Hello: {}",
                    device_id, e
                );
                e
            })?;

        let mut tls_stream = tls_stream;
        let _remote_hello = BepHandshaker::server_handshake(&mut tls_stream, device_name).await?;

        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
        let tls_pipe = TlsPipe::new(tls_stream, local_addr, peer_addr);
        let conn =
            BepConnection::new(Box::new(tls_pipe), ConnectionType::Incoming, event_tx).await?;

        conn.set_device_id(device_id);
        manager.register_connection(device_id, conn).await?;

        info!(
            "Incoming BEP connection registered for device {}",
            device_id
        );
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
