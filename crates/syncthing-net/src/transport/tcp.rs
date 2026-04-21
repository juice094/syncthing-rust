//! 原始 TCP 传输实现
//!
//! `RawTcpTransport` 是 `Transport` trait 的 TCP 实现，
//! 只负责建立原始 TCP 连接，不做 TLS 握手，不做 BEP Hello。

use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info};

use syncthing_core::{
    BoxedPipe, Result, SyncthingError, Transport, TransportListener, TransportType,
};
use syncthing_core::traits::ReliablePipe;

/// 原始 TCP 传输（不做 TLS，不做 BEP Hello）。
///
/// 这是 `Transport` trait 的 TCP 实现，只负责建立原始 TCP 连接。
/// TLS 握手和 BEP Hello 由上层（`ConnectionManager` / `BepSession`）统一处理。
#[derive(Debug)]
pub struct RawTcpTransport;

impl RawTcpTransport {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RawTcpTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Transport for RawTcpTransport {
    fn scheme(&self) -> &'static str {
        "tcp"
    }

    async fn bind(&self, addr: SocketAddr) -> Result<Box<dyn TransportListener>> {
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            SyncthingError::connection(format!("TCP bind failed on {}: {}", addr, e))
        })?;
        let actual_addr = listener
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("TCP local_addr failed: {}", e)))?;
        info!("RawTcpTransport bound to {}", actual_addr);
        Ok(Box::new(RawTcpListener {
            listener,
            local_addr: actual_addr,
        }))
    }

    async fn dial(&self, addr: SocketAddr) -> Result<BoxedPipe> {
        debug!("RawTcpTransport dialing {}", addr);
        let stream = TcpStream::connect(addr).await.map_err(|e| {
            SyncthingError::connection(format!("TCP connect to {} failed: {}", addr, e))
        })?;
        stream
            .set_nodelay(true)
            .map_err(|e| SyncthingError::connection(format!("TCP set_nodelay failed: {}", e)))?;
        let peer_addr = stream.peer_addr().map_err(|e| {
            SyncthingError::connection(format!("TCP peer_addr failed: {}", e))
        })?;
        debug!("RawTcpTransport connected to {}", peer_addr);
        Ok(Box::new(RawTcpStream { stream }))
    }
}

/// 原始 TCP 监听器
#[derive(Debug)]
pub struct RawTcpListener {
    listener: TcpListener,
    local_addr: SocketAddr,
}

#[async_trait::async_trait]
impl TransportListener for RawTcpListener {
    async fn accept(&self) -> Result<(BoxedPipe, SocketAddr)> {
        let (stream, addr) = self.listener.accept().await.map_err(|e| {
            SyncthingError::connection(format!("TCP accept failed: {}", e))
        })?;
        stream
            .set_nodelay(true)
            .map_err(|e| SyncthingError::connection(format!("TCP set_nodelay failed: {}", e)))?;
        debug!("RawTcpListener accepted connection from {}", addr);
        Ok((Box::new(RawTcpStream { stream }), addr))
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }
}

/// 原始 TCP 流，实现 ReliablePipe
#[derive(Debug)]
pub struct RawTcpStream {
    stream: TcpStream,
}

impl tokio::io::AsyncRead for RawTcpStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for RawTcpStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl ReliablePipe for RawTcpStream {
    fn local_addr(&self) -> Option<SocketAddr> {
        self.stream.local_addr().ok()
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        self.stream.peer_addr().ok()
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Tcp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_raw_tcp_transport_roundtrip() {
        let transport = RawTcpTransport::new();

        // Bind
        let listener = transport
            .bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let local_addr = listener.local_addr().unwrap();

        // Dial in background
        let dial_handle = tokio::spawn(async move {
            let t = RawTcpTransport::new();
            t.dial(local_addr).await.unwrap()
        });

        // Accept
        let (server_pipe, _client_addr) = listener.accept().await.unwrap();

        // Wait for client
        let client_pipe = dial_handle.await.unwrap();

        // Verify transport type
        assert_eq!(server_pipe.transport_type(), TransportType::Tcp);
        assert_eq!(client_pipe.transport_type(), TransportType::Tcp);
    }

    #[tokio::test]
    async fn test_raw_tcp_stream_read_write() {
        let transport = RawTcpTransport::new();
        let listener = transport
            .bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let local_addr = listener.local_addr().unwrap();

        let dial_handle = tokio::spawn(async move {
            let t = RawTcpTransport::new();
            t.dial(local_addr).await.unwrap()
        });

        let (mut server_pipe, _) = listener.accept().await.unwrap();
        let mut client_pipe = dial_handle.await.unwrap();

        // Client writes
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        client_pipe.write_all(b"hello").await.unwrap();
        client_pipe.flush().await.unwrap();

        // Server reads
        let mut buf = [0u8; 5];
        server_pipe.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");
    }
}
