//! WebSocket transport layer (optional feature: `websocket`).
//!
//! Implements WebSocket as a `Transport` to penetrate strict enterprise
//! firewalls. WebSocket traffic is usually allowed (it looks like normal web
//! traffic), making it an effective fallback when TCP+TLS is blocked.
//!
//! Two variants:
//! - `ws://` (WebSocketTransport): plaintext WebSocket, trusted LAN only
//! - `wss://` (WssWebSocketTransport): TLS-over-WebSocket, for production use

#[cfg(feature = "websocket")]
use std::net::SocketAddr;
#[cfg(feature = "websocket")]
use std::pin::Pin;
#[cfg(feature = "websocket")]
use std::sync::Arc;
#[cfg(feature = "websocket")]
use std::task::{Context, Poll};

#[cfg(feature = "websocket")]
use futures::{SinkExt, StreamExt};
#[cfg(feature = "websocket")]
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
#[cfg(feature = "websocket")]
use tokio::net::TcpListener;
#[cfg(feature = "websocket")]
use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};
#[cfg(feature = "websocket")]
use tracing::{debug, info, warn};

#[cfg(feature = "websocket")]
use syncthing_core::traits::ReliablePipe;
#[cfg(feature = "websocket")]
use syncthing_core::{
    BoxedPipe, Result, SyncthingError, Transport, TransportListener, TransportType,
};

/// WebSocket transport (plaintext ws://).
///
/// Suitable only for trusted LAN environments; use WssWebSocketTransport for
/// production/enterprise deployments.
#[cfg(feature = "websocket")]
#[derive(Debug)]
pub struct WebSocketTransport;

#[cfg(feature = "websocket")]
impl WebSocketTransport {
    /// Creates a new plaintext WebSocket transport.
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "websocket")]
impl Default for WebSocketTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "websocket")]
#[async_trait::async_trait]
impl Transport for WebSocketTransport {
    fn scheme(&self) -> &'static str {
        "ws"
    }

    async fn bind(&self, addr: SocketAddr) -> Result<Box<dyn TransportListener>> {
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            SyncthingError::connection(format!("WebSocket TCP bind failed on {}: {}", addr, e))
        })?;
        let actual_addr = listener.local_addr().map_err(|e| {
            SyncthingError::connection(format!("WebSocket local_addr failed: {}", e))
        })?;
        info!("WebSocket listener bound to {}", actual_addr);
        Ok(Box::new(WebSocketListener {
            listener,
            local_addr: actual_addr,
        }))
    }

    async fn dial(&self, addr: SocketAddr) -> Result<BoxedPipe> {
        let url = format!("ws://{}/", addr);
        debug!("WebSocket dialing {}", url);
        let (ws_stream, _response) = connect_async(&url).await.map_err(|e| {
            SyncthingError::connection(format!("WebSocket connect to {} failed: {}", url, e))
        })?;
        info!("WebSocket connected to {}", url);
        Ok(Box::new(WebSocketPipe::new(ws_stream)))
    }
}

/// WSS (WebSocket Secure) transport.
///
/// Performs the WebSocket handshake on top of standard TLS, reusing the
/// project's SyncthingTlsConfig for device authentication. `wss://` traffic
/// presents as HTTPS WebSocket and passes most enterprise firewalls.
#[cfg(feature = "websocket")]
#[derive(Clone)]
pub struct WssWebSocketTransport {
    tls_config: Arc<crate::tls::SyncthingTlsConfig>,
}

// Manual Debug impl: SyncthingTlsConfig may not implement Debug.
#[cfg(feature = "websocket")]
impl std::fmt::Debug for WssWebSocketTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WssWebSocketTransport")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "websocket")]
impl WssWebSocketTransport {
    /// Creates a new WSS transport using the given TLS configuration.
    pub fn new(tls_config: Arc<crate::tls::SyncthingTlsConfig>) -> Self {
        Self { tls_config }
    }
}

#[cfg(feature = "websocket")]
#[async_trait::async_trait]
impl Transport for WssWebSocketTransport {
    fn scheme(&self) -> &'static str {
        "wss"
    }

    async fn bind(&self, addr: SocketAddr) -> Result<Box<dyn TransportListener>> {
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            SyncthingError::connection(format!("WSS TCP bind failed on {}: {}", addr, e))
        })?;
        let actual_addr = listener
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("WSS local_addr failed: {}", e)))?;
        info!("WSS listener bound to {}", actual_addr);
        Ok(Box::new(WssListener {
            listener,
            local_addr: actual_addr,
            tls_config: Arc::clone(&self.tls_config),
        }))
    }

    async fn dial(&self, addr: SocketAddr) -> Result<BoxedPipe> {
        use tokio::net::TcpStream;
        use tokio_rustls::TlsConnector;

        let tcp = TcpStream::connect(addr).await.map_err(|e| {
            SyncthingError::connection(format!("WSS TCP connect to {} failed: {}", addr, e))
        })?;

        let client_config = self
            .tls_config
            .client_config()
            .map_err(|e| SyncthingError::Tls(format!("WSS client TLS config: {}", e)))?;
        let connector = TlsConnector::from(Arc::new(client_config));
        let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("syncthing")
            .map_err(|_| SyncthingError::Tls("invalid server name for WSS".to_string()))?;
        let tls_stream = connector
            .connect(server_name, tcp)
            .await
            .map_err(|e| SyncthingError::Tls(format!("WSS TLS handshake failed: {}", e)))?;

        let url = format!("wss://{}/", addr);
        let ws_stream = tokio_tungstenite::client_async(&url, tls_stream)
            .await
            .map_err(|e| {
                SyncthingError::connection(format!("WSS WebSocket handshake failed: {}", e))
            })?
            .0;

        info!("WSS connected to {}", addr);
        Ok(Box::new(WebSocketPipe::new(ws_stream)))
    }
}

/// WSS listener: TLS accept -> WebSocket upgrade.
#[cfg(feature = "websocket")]
struct WssListener {
    listener: TcpListener,
    local_addr: SocketAddr,
    tls_config: Arc<crate::tls::SyncthingTlsConfig>,
}

#[cfg(feature = "websocket")]
impl std::fmt::Debug for WssListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WssListener")
            .field("local_addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "websocket")]
#[async_trait::async_trait]
impl TransportListener for WssListener {
    async fn accept(&self) -> Result<(BoxedPipe, SocketAddr)> {
        let (stream, peer_addr) = self
            .listener
            .accept()
            .await
            .map_err(|e| SyncthingError::connection(format!("WSS TCP accept failed: {}", e)))?;

        let server_config = self
            .tls_config
            .server_config()
            .map_err(|e| SyncthingError::Tls(format!("WSS server TLS config: {}", e)))?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
        let tls_stream = acceptor
            .accept(stream)
            .await
            .map_err(|e| SyncthingError::Tls(format!("WSS TLS accept failed: {}", e)))?;

        let ws_stream = accept_async(tls_stream).await.map_err(|e| {
            SyncthingError::connection(format!("WSS WebSocket accept failed: {}", e))
        })?;

        debug!("WSS accepted connection from {}", peer_addr);
        Ok((Box::new(WebSocketPipe::new(ws_stream)), peer_addr))
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }
}

/// WebSocket listener.
#[cfg(feature = "websocket")]
#[derive(Debug)]
pub struct WebSocketListener {
    listener: TcpListener,
    local_addr: SocketAddr,
}

#[cfg(feature = "websocket")]
#[async_trait::async_trait]
impl TransportListener for WebSocketListener {
    async fn accept(&self) -> Result<(BoxedPipe, SocketAddr)> {
        let (stream, peer_addr) = self.listener.accept().await.map_err(|e| {
            SyncthingError::connection(format!("WebSocket TCP accept failed: {}", e))
        })?;
        let ws_stream = accept_async(stream).await.map_err(|e| {
            SyncthingError::connection(format!("WebSocket handshake failed: {}", e))
        })?;
        debug!("WebSocket accepted connection from {}", peer_addr);
        Ok((Box::new(WebSocketPipe::new(ws_stream)), peer_addr))
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }
}

/// Merges one end of each of two DuplexStreams into a single AsyncRead + AsyncWrite.
/// Reads come from one DuplexStream's read half, writes from the other's write half.
#[cfg(feature = "websocket")]
struct MergedPipe {
    read: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
}

#[cfg(feature = "websocket")]
impl AsyncRead for MergedPipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}

#[cfg(feature = "websocket")]
impl AsyncWrite for MergedPipe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}

/// WebSocket -> byte stream bridge.
///
/// Internally uses two `tokio::io::duplex` pipes:
/// - `ws_to_bep`: WebSocket Binary received -> bridge write -> BEP read
/// - `bep_to_ws`: BEP write -> bridge read -> WebSocket Binary send
///
/// What the BEP layer sees is a `MergedPipe` combining one end of each pipe.
#[cfg(feature = "websocket")]
pub struct WebSocketPipe {
    inner: MergedPipe,
    local_addr: Option<SocketAddr>,
    peer_addr: Option<SocketAddr>,
}

#[cfg(feature = "websocket")]
impl WebSocketPipe {
    fn new<S>(ws_stream: tokio_tungstenite::WebSocketStream<S>) -> Self
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        let (bep_read, mut ws_to_bep) = tokio::io::duplex(64 * 1024);
        let (bep_to_ws, mut ws_write) = tokio::io::duplex(64 * 1024);

        tokio::spawn(async move {
            let recv_to_duplex = async {
                while let Some(msg) = ws_rx.next().await {
                    match msg {
                        Ok(Message::Binary(data)) if ws_to_bep.write_all(&data).await.is_err() => {
                            break;
                        }
                        Ok(Message::Close(_)) => break,
                        Err(e) => {
                            warn!("WebSocket receive error: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }
            };

            let duplex_to_send = async {
                let mut buf = vec![0u8; 4096];
                loop {
                    match ws_write.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if ws_tx
                                .send(Message::Binary(buf[..n].to_vec().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            };

            tokio::select! {
                _ = recv_to_duplex => {},
                _ = duplex_to_send => {},
            }
        });

        let (read, _) = tokio::io::split(bep_read);
        let (_, write) = tokio::io::split(bep_to_ws);

        Self {
            inner: MergedPipe { read, write },
            local_addr: None,
            peer_addr: None,
        }
    }
}

#[cfg(feature = "websocket")]
impl AsyncRead for WebSocketPipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

#[cfg(feature = "websocket")]
impl AsyncWrite for WebSocketPipe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(feature = "websocket")]
impl ReliablePipe for WebSocketPipe {
    fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr
    }

    fn transport_type(&self) -> TransportType {
        TransportType::WebSocket
    }
}

/// Placeholder when the `websocket` feature is disabled.
#[cfg(not(feature = "websocket"))]
pub struct WebSocketTransport;

#[cfg(not(feature = "websocket"))]
impl WebSocketTransport {
    /// Creates a new placeholder transport.
    pub fn new() -> Self {
        Self
    }
}
