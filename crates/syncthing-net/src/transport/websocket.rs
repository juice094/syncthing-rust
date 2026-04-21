//! WebSocket 传输层（可选 feature：`websocket`）
//!
//! 将 WebSocket 作为 `Transport` trait 的实现，用于穿透严格的企业防火墙。
//! WebSocket 流量通常被防火墙允许（因为看起来像正常的 Web 流量），
//! 因此是 TCP+TLS 被封锁时的有效 fallback。
//!
//! 当前实现使用 `tokio-tungstenite`，将 WebSocket binary frames 桥接为
//! 连续的字节流（通过内部 duplex 缓冲）。

#[cfg(feature = "websocket")]
use std::net::SocketAddr;
#[cfg(feature = "websocket")]
use std::pin::Pin;
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
use syncthing_core::{
    BoxedPipe, Result, SyncthingError, Transport, TransportListener, TransportType,
};
#[cfg(feature = "websocket")]
use syncthing_core::traits::ReliablePipe;

/// WebSocket 传输层。
///
/// 通过 `ws://` 建立连接。`wss://`（TLS over WebSocket）可后续添加。
#[cfg(feature = "websocket")]
#[derive(Debug)]
pub struct WebSocketTransport;

#[cfg(feature = "websocket")]
impl WebSocketTransport {
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

/// WebSocket 监听器
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

/// 合并两个 DuplexStream 的各一端，形成单一的 AsyncRead + AsyncWrite。
/// read 来自一个 DuplexStream 的读端，write 来自另一个 DuplexStream 的写端。
#[cfg(feature = "websocket")]
struct MergedPipe {
    read: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
}

#[cfg(feature = "websocket")]
impl AsyncRead for MergedPipe {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}

#[cfg(feature = "websocket")]
impl AsyncWrite for MergedPipe {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}

/// WebSocket → 字节流桥接。
///
/// 内部使用两个 `tokio::io::duplex` 管道：
/// - `ws_to_bep`：WebSocket 收到 Binary → 桥接写入 → BEP 读取
/// - `bep_to_ws`：BEP 写入 → 桥接读取 → WebSocket 发送 Binary
/// 最终暴露给 BEP 层的是 `MergedPipe`（组合两个管道的各一端）。
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

        // duplex1: WebSocket → BEP
        let (bep_read, mut ws_to_bep) = tokio::io::duplex(64 * 1024);
        // duplex2: BEP → WebSocket
        let (bep_to_ws, mut ws_write) = tokio::io::duplex(64 * 1024);

        // 启动桥接任务
        tokio::spawn(async move {
            // WebSocket 接收 → ws_to_bep 写入
            let recv_to_duplex = async {
                while let Some(msg) = ws_rx.next().await {
                    match msg {
                        Ok(Message::Binary(data)) => {
                            if ws_to_bep.write_all(&data).await.is_err() {
                                break;
                            }
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

            // ws_write 读取 → WebSocket 发送
            let duplex_to_send = async {
                let mut buf = vec![0u8; 4096];
                loop {
                    match ws_write.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if ws_tx.send(Message::Binary(buf[..n].to_vec())).await.is_err() {
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
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

#[cfg(feature = "websocket")]
impl AsyncWrite for WebSocketPipe {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
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

/// 未启用 `websocket` feature 时的占位模块
#[cfg(not(feature = "websocket"))]
pub struct WebSocketTransport;

#[cfg(not(feature = "websocket"))]
impl WebSocketTransport {
    pub fn new() -> Self {
        Self
    }
}
