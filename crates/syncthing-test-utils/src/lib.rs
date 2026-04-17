//! Test utilities for syncthing-rust
//!
//! Provides in-memory primitives for deterministic testing of the network
//! and protocol layers without spawning real TCP sockets or processes.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// An in-memory bidirectional pipe for testing BEP and transport layers.
///
/// Each end of the pair implements [`AsyncRead`] and [`AsyncWrite`], making
/// it directly usable as a byte stream underneath `BepConnection`.
pub struct MemoryPipe {
    inner: tokio::io::DuplexStream,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    closed: bool,
}

impl MemoryPipe {
    fn new(
        inner: tokio::io::DuplexStream,
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
    ) -> Self {
        Self {
            inner,
            local_addr,
            peer_addr,
            closed: false,
        }
    }

    /// Close the pipe.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Local endpoint address (dummy, for API compatibility).
    pub fn local_addr(&self) -> Option<SocketAddr> {
        Some(self.local_addr)
    }

    /// Peer endpoint address (dummy, for API compatibility).
    pub fn peer_addr(&self) -> Option<SocketAddr> {
        Some(self.peer_addr)
    }
}

impl AsyncRead for MemoryPipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.closed {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for MemoryPipe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.closed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "MemoryPipe closed",
            )));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl syncthing_core::traits::ReliablePipe for MemoryPipe {
    fn local_addr(&self) -> Option<SocketAddr> {
        Some(self.local_addr)
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        Some(self.peer_addr)
    }

    fn transport_type(&self) -> syncthing_core::traits::TransportType {
        syncthing_core::traits::TransportType::Memory
    }
}

/// Create a pair of connected [`MemoryPipe`]s.
///
/// `max_buf_size` controls the internal backpressure buffer of the underlying
/// `tokio::io::duplex`.  Data written to one side is readable from the other.
pub fn memory_pipe_pair(max_buf_size: usize) -> (MemoryPipe, MemoryPipe) {
    let (a, b) = tokio::io::duplex(max_buf_size);

    let addr_a: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let addr_b: SocketAddr = "127.0.0.1:2".parse().unwrap();

    (
        MemoryPipe::new(a, addr_a, addr_b),
        MemoryPipe::new(b, addr_b, addr_a),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_memory_pipe_roundtrip() {
        let (mut a, mut b) = memory_pipe_pair(1024);

        a.write_all(b"hello").await.unwrap();
        a.flush().await.unwrap();

        let mut buf = [0u8; 5];
        b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");

        b.write_all(b"world").await.unwrap();
        b.flush().await.unwrap();

        let mut buf2 = [0u8; 5];
        a.read_exact(&mut buf2).await.unwrap();
        assert_eq!(&buf2, b"world");
    }
}
