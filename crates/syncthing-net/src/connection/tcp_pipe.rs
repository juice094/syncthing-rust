//! TCP/TLS unified bi-directional stream wrapper.
//!
//! `TcpBiStream` is a transport-layer abstraction over plain TCP, client TLS,
//! and server TLS streams. Implements `tokio::io::{AsyncRead, AsyncWrite}` and
//! `syncthing_core::traits::ReliablePipe`.

use std::net::SocketAddr;

use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;

/// TCP 连接统一类型（支持明文 TCP 或 TLS）
#[derive(Debug)]
pub enum TcpBiStream {
    /// 明文 TCP 流
    Plain(TcpStream),
    /// 客户端 TLS 流
    Client(ClientTlsStream<TcpStream>),
    /// 服务端 TLS 流
    Server(ServerTlsStream<TcpStream>),
}

impl TcpBiStream {
    pub(super) fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        match self {
            TcpBiStream::Plain(s) => s.peer_addr(),
            TcpBiStream::Client(s) => s.get_ref().0.peer_addr(),
            TcpBiStream::Server(s) => s.get_ref().0.peer_addr(),
        }
    }

    pub(super) fn local_addr(&self) -> std::io::Result<SocketAddr> {
        match self {
            TcpBiStream::Plain(s) => s.local_addr(),
            TcpBiStream::Client(s) => s.get_ref().0.local_addr(),
            TcpBiStream::Server(s) => s.get_ref().0.local_addr(),
        }
    }
}

impl tokio::io::AsyncRead for TcpBiStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            TcpBiStream::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            TcpBiStream::Client(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            TcpBiStream::Server(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for TcpBiStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match &mut *self {
            TcpBiStream::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            TcpBiStream::Client(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            TcpBiStream::Server(s) => std::pin::Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            TcpBiStream::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            TcpBiStream::Client(s) => std::pin::Pin::new(s).poll_flush(cx),
            TcpBiStream::Server(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            TcpBiStream::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            TcpBiStream::Client(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            TcpBiStream::Server(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }
}

impl syncthing_core::traits::ReliablePipe for TcpBiStream {
    fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr().ok()
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr().ok()
    }

    fn transport_type(&self) -> syncthing_core::traits::TransportType {
        syncthing_core::traits::TransportType::Tcp
    }
}
