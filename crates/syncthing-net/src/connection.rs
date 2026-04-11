//! BEP 连接实现
//!
//! 实现Syncthing BEP协议的连接层
//! 参考: syncthing/lib/connections/*.go

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use parking_lot::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use syncthing_core::{ConnectionState, ConnectionStats, ConnectionType, DeviceId, Result, SyncthingError};
use crate::protocol::{MessageHeader, MessageType};

/// 默认消息超时
pub const DEFAULT_MESSAGE_TIMEOUT: Duration = Duration::from_secs(60);

/// 心跳间隔
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(90);

/// 连接事件
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// 连接已建立
    Connected { device_id: DeviceId },
    /// 握手完成
    HandshakeComplete { device_id: DeviceId },
    /// 消息收到
    MessageReceived { device_id: DeviceId, msg_type: MessageType },
    /// 连接断开
    Disconnected { reason: String },
    /// 错误
    Error { error: String },
}

/// 连接内部状态
struct ConnectionInner {
    /// 连接ID
    pub id: uuid::Uuid,
    /// 远程地址
    pub remote_addr: SocketAddr,
    /// 本地地址
    pub local_addr: SocketAddr,
    /// 连接状态
    pub state: RwLock<ConnectionState>,
    /// 连接统计
    pub stats: RwLock<ConnectionStats>,
    /// 连接类型
    pub conn_type: ConnectionType,
    /// 关联的设备ID
    pub device_id: RwLock<Option<DeviceId>>,
    /// 最后ping时间
    pub last_ping: RwLock<Instant>,
    /// 最后pong时间
    pub last_pong: RwLock<Instant>,
}

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
    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        match self {
            TcpBiStream::Plain(s) => s.peer_addr(),
            TcpBiStream::Client(s) => s.get_ref().0.peer_addr(),
            TcpBiStream::Server(s) => s.get_ref().0.peer_addr(),
        }
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
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

    fn poll_flush(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            TcpBiStream::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            TcpBiStream::Client(s) => std::pin::Pin::new(s).poll_flush(cx),
            TcpBiStream::Server(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            TcpBiStream::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            TcpBiStream::Client(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            TcpBiStream::Server(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// BEP 连接包装器
///
/// 封装底层的TCP连接，处理BEP协议细节
pub struct BepConnection {
    /// 内部状态
    inner: Arc<ConnectionInner>,
    /// 底层TCP流
    stream: Arc<Mutex<TcpBiStream>>,
    /// 消息发送通道
    message_tx: mpsc::UnboundedSender<Message>,
    /// 事件发送器
    event_tx: mpsc::UnboundedSender<ConnectionEvent>,
    /// 关闭信号
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,
    /// 接收消息通道发送端（供读取任务使用）
    incoming_tx: mpsc::UnboundedSender<(MessageType, Bytes)>,
    /// 接收消息通道接收端
    incoming_rx: Arc<Mutex<mpsc::UnboundedReceiver<(MessageType, Bytes)>>>,
}

/// 内部消息结构
#[derive(Debug)]
struct Message {
    pub header: MessageHeader,
    pub payload: Bytes,
}

impl BepConnection {
    /// 从TCP流创建新连接
    pub async fn new(
        stream: TcpBiStream,
        conn_type: ConnectionType,
        event_tx: mpsc::UnboundedSender<ConnectionEvent>,
    ) -> Result<Arc<Self>> {
        let remote_addr = stream.peer_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get peer addr: {}", e)))?;
        let local_addr = stream.local_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get local addr: {}", e)))?;
        
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        
        let id = uuid::Uuid::new_v4();
        
        let conn = Arc::new(Self {
            inner: Arc::new(ConnectionInner {
                id,
                remote_addr,
                local_addr,
                state: RwLock::new(ConnectionState::Connected),
                stats: RwLock::new(ConnectionStats {
                    connected_at: Some(chrono::Utc::now()),
                    ..Default::default()
                }),
                conn_type,
                device_id: RwLock::new(None),
                last_ping: RwLock::new(Instant::now()),
                last_pong: RwLock::new(Instant::now()),
            }),
            stream: Arc::new(Mutex::new(stream)),
            message_tx,
            event_tx,
            shutdown_tx: RwLock::new(Some(shutdown_tx)),
            incoming_tx,
            incoming_rx: Arc::new(Mutex::new(incoming_rx)),
        });
        
        // 启动连接处理任务
        let conn_clone = Arc::clone(&conn);
        let message_rx = Arc::new(Mutex::new(message_rx));
        tokio::spawn(async move {
            if let Err(e) = conn_clone.run(shutdown_rx, message_rx).await {
                error!("Connection {} error: {}", id, e);
            }
        });
        
        Ok(conn)
    }
    
    /// 获取连接ID
    pub fn id(&self) -> uuid::Uuid {
        self.inner.id
    }
    
    /// 获取远程地址
    pub fn remote_addr(&self) -> SocketAddr {
        self.inner.remote_addr
    }
    
    /// 获取本地地址
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }
    
    /// 获取当前状态
    pub fn state(&self) -> ConnectionState {
        *self.inner.state.read()
    }
    
    /// 设置状态
    pub fn set_state(&self, state: ConnectionState) {
        *self.inner.state.write() = state;
    }
    
    /// 获取设备ID
    pub fn device_id(&self) -> Option<DeviceId> {
        *self.inner.device_id.read()
    }
    
    /// 设置设备ID
    pub fn set_device_id(&self, device_id: DeviceId) {
        *self.inner.device_id.write() = Some(device_id);
        
        // 通知连接建立
        let _ = self.event_tx.send(ConnectionEvent::Connected {
            device_id,
        });
    }
    
    /// 获取连接类型
    pub fn connection_type(&self) -> ConnectionType {
        self.inner.conn_type
    }
    
    /// 获取统计信息
    pub fn stats(&self) -> ConnectionStats {
        self.inner.stats.read().clone()
    }
    
    /// 更新统计信息
    fn update_stats<F>(&self, f: F)
    where
        F: FnOnce(&mut ConnectionStats),
    {
        let mut stats = self.inner.stats.write();
        f(&mut *stats);
    }
    
    /// 发送消息
    pub async fn send_message(&self, msg_type: MessageType, payload: Bytes) -> Result<()> {
        let header = MessageHeader {
            message_type: msg_type,
            message_id: 0,
            compressed: false,
        };
        
        let msg = Message { header, payload };
        self.message_tx
            .send(msg)
            .map_err(|_| SyncthingError::ConnectionClosed)?;
        
        self.update_stats(|s| {
            s.messages_sent += 1;
            s.last_activity = Some(chrono::Utc::now());
        });
        
        Ok(())
    }
    
    /// 发送Ping（BEP 没有独立的 Pong，收到 Ping 后通常回一个 Ping）
    pub async fn send_ping(&self) -> Result<()> {
        self.send_message(MessageType::Ping, Bytes::new()).await
    }
    
    /// 发送Pong（兼容旧调用，实际发送 Ping）
    pub async fn send_pong(&self) -> Result<()> {
        self.send_message(MessageType::Ping, Bytes::new()).await
    }

    pub async fn send_cluster_config(&self, cc: &bep_protocol::messages::ClusterConfig) -> Result<()> {
        let payload = bep_protocol::messages::encode_message(cc)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        self.send_message(MessageType::ClusterConfig, payload).await
    }

    pub async fn send_index(&self, index: &syncthing_core::Index) -> Result<()> {
        let wire: bep_protocol::messages::Index = index.clone().into();
        let payload = bep_protocol::messages::encode_message(&wire)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        self.send_message(MessageType::Index, payload).await
    }

    pub async fn send_index_update(&self, update: &syncthing_core::IndexUpdate) -> Result<()> {
        let wire: bep_protocol::messages::IndexUpdate = update.clone().into();
        let payload = bep_protocol::messages::encode_message(&wire)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        self.send_message(MessageType::IndexUpdate, payload).await
    }
    
    /// 关闭连接
    pub async fn close(&self) -> Result<()> {
        info!("Closing connection {}", self.id());
        
        self.set_state(ConnectionState::Disconnecting);
        
        // 触发关闭信号
        if let Some(tx) = self.shutdown_tx.write().take() {
            let _ = tx.send(());
        }
        
        self.set_state(ConnectionState::Disconnected);
        
        // 发送断开事件
        let _ = self.event_tx.send(ConnectionEvent::Disconnected {
            reason: "connection closed".to_string(),
        });
        
        Ok(())
    }
    
    /// 接收 BEP 消息
    pub async fn recv_message(&self) -> Result<(MessageType, Bytes)> {
        let mut rx = self.incoming_rx.lock().await;
        rx.recv().await.ok_or_else(|| SyncthingError::ConnectionClosed)
    }
    
    /// 主运行循环
    async fn run(
        &self,
        mut shutdown_rx: oneshot::Receiver<()>,
        message_rx: Arc<Mutex<mpsc::UnboundedReceiver<Message>>>,
    ) -> Result<()> {
        // 启动读取任务
        let read_handle = self.spawn_read_task();
        
        // 启动写入任务
        let write_handle = self.spawn_write_task(message_rx);
        
        // 启动心跳任务
        let heartbeat_handle = self.spawn_heartbeat_task();
        
        // 等待关闭信号
        tokio::select! {
            _ = &mut shutdown_rx => {
                debug!("Connection {} received shutdown signal", self.id());
            }
            _ = read_handle => {
                debug!("Connection {} read task ended", self.id());
            }
            _ = write_handle => {
                debug!("Connection {} write task ended", self.id());
            }
        }
        
        heartbeat_handle.abort();
        
        Ok(())
    }
    
    /// 启动读取任务
    fn spawn_read_task(&self) -> tokio::task::JoinHandle<Result<()>> {
        let stream = Arc::clone(&self.stream);
        let event_tx = self.event_tx.clone();
        let incoming_tx = self.incoming_tx.clone();
        let inner = Arc::clone(&self.inner);

        tokio::spawn(async move {
            loop {
                let mut stream_guard = stream.lock().await;

                // 读取 2 字节 header length
                let mut hdr_len_buf = [0u8; 2];
                match timeout(DEFAULT_MESSAGE_TIMEOUT, stream_guard.read_exact(&mut hdr_len_buf)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => {
                        drop(stream_guard);
                        continue;
                    }
                }
                let hdr_len = u16::from_be_bytes(hdr_len_buf) as usize;

                // 读取 header 字节
                let mut hdr_buf = vec![0u8; hdr_len];
                match timeout(DEFAULT_MESSAGE_TIMEOUT, stream_guard.read_exact(&mut hdr_buf)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => return Err(SyncthingError::timeout("header read timeout")),
                }

                // 读取 4 字节 message length
                let mut msg_len_buf = [0u8; 4];
                match timeout(DEFAULT_MESSAGE_TIMEOUT, stream_guard.read_exact(&mut msg_len_buf)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => return Err(SyncthingError::timeout("message length read timeout")),
                }
                let msg_len = u32::from_be_bytes(msg_len_buf) as usize;

                // 读取 message 字节
                let mut msg_buf = vec![0u8; msg_len];
                match timeout(DEFAULT_MESSAGE_TIMEOUT, stream_guard.read_exact(&mut msg_buf)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => return Err(SyncthingError::timeout("message read timeout")),
                }

                let bytes_received = 2 + hdr_len + 4 + msg_len;
                let mut stats = inner.stats.write();
                stats.bytes_received += bytes_received as u64;
                stats.last_activity = Some(chrono::Utc::now());
                stats.messages_received += 1;
                drop(stats);
                drop(stream_guard);

                // 解码 BEP Header
                let bep_header = match <bep_protocol::messages::Header as prost::Message>::decode(&hdr_buf[..]) {
                    Ok(h) => h,
                    Err(e) => return Err(SyncthingError::protocol(format!("decode header failed: {}", e))),
                };

                let header = match MessageHeader::from_bep_header(&bep_header) {
                    Some(h) => h,
                    None => return Err(SyncthingError::protocol(format!("unknown message type: {}", bep_header.r#type))),
                };

                debug!("Received message: {:?}", header.message_type);

                if let Some(device_id) = *inner.device_id.read() {
                    let _ = event_tx.send(ConnectionEvent::MessageReceived {
                        device_id,
                        msg_type: header.message_type,
                    });
                }

                let _ = incoming_tx.send((header.message_type, Bytes::from(msg_buf)));
            }
        })
    }

    /// 启动写入任务
    fn spawn_write_task(
        &self,
        message_rx: Arc<Mutex<mpsc::UnboundedReceiver<Message>>>,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let stream = Arc::clone(&self.stream);
        let inner = Arc::clone(&self.inner);

        tokio::spawn(async move {
            let mut rx = message_rx.lock().await;

            while let Some(msg) = rx.recv().await {
                let bep_header = msg.header.to_bep_header();
                let mut hdr_buf = Vec::new();
                if let Err(e) = <bep_protocol::messages::Header as prost::Message>::encode(&bep_header, &mut hdr_buf) {
                    return Err(SyncthingError::Serialization(e.to_string()));
                }
                let hdr_len = hdr_buf.len();
                let msg_len = msg.payload.len();

                let mut stream_guard = stream.lock().await;

                // header length (2 bytes)
                if let Err(e) = stream_guard.write_all(&(hdr_len as u16).to_be_bytes()).await {
                    return Err(SyncthingError::Io(e));
                }
                // header
                if let Err(e) = stream_guard.write_all(&hdr_buf).await {
                    return Err(SyncthingError::Io(e));
                }
                // message length (4 bytes)
                if let Err(e) = stream_guard.write_all(&(msg_len as u32).to_be_bytes()).await {
                    return Err(SyncthingError::Io(e));
                }
                // payload
                if !msg.payload.is_empty() {
                    if let Err(e) = stream_guard.write_all(&msg.payload).await {
                        return Err(SyncthingError::Io(e));
                    }
                }
                if let Err(e) = stream_guard.flush().await {
                    return Err(SyncthingError::Io(e));
                }
                drop(stream_guard);

                let mut stats = inner.stats.write();
                stats.bytes_sent += (2 + hdr_len + 4 + msg_len) as u64;
                stats.messages_sent += 1;
                stats.last_activity = Some(chrono::Utc::now());
            }

            Ok(())
        })
    }
    
    /// 启动心跳任务
    fn spawn_heartbeat_task(&self) -> tokio::task::JoinHandle<()> {
        let inner = Arc::clone(&self.inner);
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            
            loop {
                interval.tick().await;
                
                let last_pong = *inner.last_pong.read();
                if last_pong.elapsed() > HEARTBEAT_INTERVAL * 3 {
                    // 心跳超时，应该断开连接
                    warn!("Heartbeat timeout, closing connection");
                    break;
                }
            }
        })
    }
    
    /// 检查连接是否活跃
    pub fn is_alive(&self) -> bool {
        matches!(
            self.state(),
            ConnectionState::Connected
                | ConnectionState::TlsHandshakeComplete
                | ConnectionState::ProtocolHandshakeComplete
                | ConnectionState::ClusterConfigComplete
        )
    }
}

impl Drop for BepConnection {
    fn drop(&mut self) {
        // 确保连接被关闭
        if self.is_alive() {
            let _ = self.event_tx.send(ConnectionEvent::Disconnected {
                reason: "connection dropped".to_string(),
            });
        }
    }
}

// ============================================
// Iroh BEP Connection
// ============================================

#[cfg(feature = "iroh")]
use bep_protocol::handshake::{recv_hello, send_hello};
#[cfg(feature = "iroh")]
use bep_protocol::messages::Hello as BepHello;
#[cfg(feature = "iroh")]
use std::pin::Pin;
#[cfg(feature = "iroh")]
use std::task::{Context, Poll};
#[cfg(feature = "iroh")]
use tokio::io::ReadBuf;
#[cfg(feature = "iroh")]
use tokio_rustls::client::TlsStream as ClientTlsStream;
#[cfg(feature = "iroh")]
use tokio_rustls::server::TlsStream as ServerTlsStream;
#[cfg(feature = "iroh")]
use crate::tls::SyncthingTlsConfig;

/// BEP ALPN identifier for iroh transport
#[cfg(feature = "iroh")]
pub const BEP_ALPN: &[u8] = b"syncthing-bep/1.0";

/// 包装 iroh 双向流为单个 AsyncRead + AsyncWrite
#[cfg(feature = "iroh")]
pub struct IrohBiStream {
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
}

#[cfg(feature = "iroh")]
impl tokio::io::AsyncRead for IrohBiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

#[cfg(feature = "iroh")]
impl tokio::io::AsyncWrite for IrohBiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        tokio::io::AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        tokio::io::AsyncWrite::poll_shutdown(Pin::new(&mut self.send), cx)
    }
}

/// 统一客户端/服务器 TLS 流类型
#[cfg(feature = "iroh")]
enum TlsBiStream {
    Client(ClientTlsStream<IrohBiStream>),
    Server(ServerTlsStream<IrohBiStream>),
}

#[cfg(feature = "iroh")]
impl tokio::io::AsyncRead for TlsBiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Client(s) => Pin::new(s).poll_read(cx, buf),
            Self::Server(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

#[cfg(feature = "iroh")]
impl tokio::io::AsyncWrite for TlsBiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            Self::Client(s) => Pin::new(s).poll_write(cx, buf),
            Self::Server(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Client(s) => Pin::new(s).poll_flush(cx),
            Self::Server(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Client(s) => Pin::new(s).poll_shutdown(cx),
            Self::Server(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Iroh-backed BEP connection
///
/// 在 iroh QUIC 双向流之上建立 TLS 隧道，再运行 BEP 协议：
/// - 先完成 tokio_rustls 握手
/// - 然后从对端证书提取 DeviceId
/// - 最后通过 TLS 流交换 BEP Hello
#[cfg(feature = "iroh")]
pub struct IrohBepConnection {
    id: uuid::Uuid,
    stream: Arc<Mutex<TlsBiStream>>,
    state: RwLock<ConnectionState>,
    stats: RwLock<ConnectionStats>,
    conn_type: ConnectionType,
    device_id: RwLock<Option<DeviceId>>,
    remote_addr: String,
    local_addr: String,
    remote_hello: RwLock<Option<BepHello>>,
}

#[cfg(feature = "iroh")]
impl IrohBepConnection {
    /// 创建新的出站 iroh BEP 连接（客户端模式：先发 Hello 再收 Hello）
    pub async fn connect(
        conn: iroh::endpoint::Connection,
        tls_config: &SyncthingTlsConfig,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<(Arc<Self>, BepHello)> {
        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| SyncthingError::connection(format!("open_bi failed: {}", e)))?;

        let remote_addr = format!("{:?}", conn.remote_id());
        let local_addr = "iroh".to_string();

        let bi_stream = IrohBiStream { send, recv };
        let (tls_stream, device_id) = crate::tls::connect_tls_stream(
            bi_stream,
            tls_config,
            None,
        ).await?;

        let id = uuid::Uuid::new_v4();

        let this = Arc::new(Self {
            id,
            stream: Arc::new(Mutex::new(TlsBiStream::Client(tls_stream))),
            state: RwLock::new(ConnectionState::TlsHandshakeComplete),
            stats: RwLock::new(ConnectionStats {
                connected_at: Some(chrono::Utc::now()),
                ..Default::default()
            }),
            remote_addr,
            local_addr,
            conn_type: ConnectionType::Outgoing,
            device_id: RwLock::new(Some(device_id)),
            remote_hello: RwLock::new(None),
        });

        let our_hello = BepHello {
            device_name: device_name.to_string(),
            client_name: client_name.to_string(),
            client_version: client_version.to_string(),
            num_connections: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        };

        {
            let mut stream = this.stream.lock().await;
            send_hello(&mut *stream, &our_hello).await?;
        }
        let remote_hello = {
            let mut stream = this.stream.lock().await;
            let remote_hello = recv_hello(&mut *stream).await?;
            remote_hello
        };
        *this.remote_hello.write() = Some(remote_hello.clone());
        this.set_state(ConnectionState::ProtocolHandshakeComplete);
        Ok((this, remote_hello))
    }

    /// 接受新的入站 iroh BEP 连接（服务端模式：先收 Hello 再发 Hello）
    pub async fn accept(
        conn: iroh::endpoint::Connection,
        tls_config: &SyncthingTlsConfig,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<(Arc<Self>, BepHello)> {
        let (send, recv) = conn
            .accept_bi()
            .await
            .map_err(|e| SyncthingError::connection(format!("accept_bi failed: {}", e)))?;

        let remote_addr = format!("{:?}", conn.remote_id());
        let local_addr = "iroh".to_string();

        let bi_stream = IrohBiStream { send, recv };
        let (tls_stream, device_id) = crate::tls::accept_tls_stream(
            bi_stream,
            tls_config,
        ).await?;

        let id = uuid::Uuid::new_v4();

        let this = Arc::new(Self {
            id,
            stream: Arc::new(Mutex::new(TlsBiStream::Server(tls_stream))),
            state: RwLock::new(ConnectionState::TlsHandshakeComplete),
            stats: RwLock::new(ConnectionStats {
                connected_at: Some(chrono::Utc::now()),
                ..Default::default()
            }),
            remote_addr,
            local_addr,
            conn_type: ConnectionType::Incoming,
            device_id: RwLock::new(Some(device_id)),
            remote_hello: RwLock::new(None),
        });

        let our_hello = BepHello {
            device_name: device_name.to_string(),
            client_name: client_name.to_string(),
            client_version: client_version.to_string(),
            num_connections: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        };

        let remote_hello = {
            let mut stream = this.stream.lock().await;
            let remote_hello = recv_hello(&mut *stream).await?;
            *this.remote_hello.write() = Some(remote_hello.clone());
            remote_hello
        };
        {
            let mut stream = this.stream.lock().await;
            send_hello(&mut *stream, &our_hello).await?;
        }
        this.set_state(ConnectionState::ProtocolHandshakeComplete);
        Ok((this, remote_hello))
    }

    pub fn id(&self) -> uuid::Uuid {
        self.id
    }

    pub fn remote_addr(&self) -> &str {
        &self.remote_addr
    }

    pub fn local_addr(&self) -> &str {
        &self.local_addr
    }

    pub fn state(&self) -> ConnectionState {
        *self.state.read()
    }

    fn set_state(&self, state: ConnectionState) {
        *self.state.write() = state;
    }

    pub fn device_id(&self) -> Option<DeviceId> {
        *self.device_id.read()
    }

    pub fn set_device_id(&self, device_id: DeviceId) {
        *self.device_id.write() = Some(device_id);
    }

    pub fn remote_hello(&self) -> Option<BepHello> {
        self.remote_hello.read().clone()
    }

    pub fn connection_type(&self) -> ConnectionType {
        self.conn_type
    }

    pub fn stats(&self) -> ConnectionStats {
        self.stats.read().clone()
    }

    fn update_stats<F>(&self, f: F)
    where
        F: FnOnce(&mut ConnectionStats),
    {
        let mut stats = self.stats.write();
        f(&mut *stats);
    }

    pub fn is_alive(&self) -> bool {
        matches!(
            self.state(),
            ConnectionState::Connected
                | ConnectionState::TlsHandshakeComplete
                | ConnectionState::ProtocolHandshakeComplete
                | ConnectionState::ClusterConfigComplete
        )
    }

    pub fn is_hello_complete(&self) -> bool {
        matches!(
            self.state(),
            ConnectionState::ProtocolHandshakeComplete | ConnectionState::ClusterConfigComplete
        )
    }

    /// 发送 BEP 消息（标准 BEP 帧格式）
    pub async fn send_message(&self, msg_type: MessageType, payload: Bytes) -> Result<()> {
        let header = MessageHeader {
            message_type: msg_type,
            message_id: 0,
            compressed: false,
        };
        let bep_header = header.to_bep_header();
        let mut hdr_buf = Vec::new();
        <bep_protocol::messages::Header as prost::Message>::encode(&bep_header, &mut hdr_buf)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        let hdr_len = hdr_buf.len();
        let msg_len = payload.len();

        let mut stream = self.stream.lock().await;
        stream.write_all(&(hdr_len as u16).to_be_bytes())
            .await
            .map_err(|e| SyncthingError::connection(format!("write failed: {}", e)))?;
        stream.write_all(&hdr_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("write failed: {}", e)))?;
        stream.write_all(&(msg_len as u32).to_be_bytes())
            .await
            .map_err(|e| SyncthingError::connection(format!("write failed: {}", e)))?;
        if !payload.is_empty() {
            stream.write_all(&payload)
                .await
                .map_err(|e| SyncthingError::connection(format!("write failed: {}", e)))?;
        }
        drop(stream);

        self.update_stats(|s| {
            s.messages_sent += 1;
            s.bytes_sent += (2 + hdr_len + 4 + msg_len) as u64;
            s.last_activity = Some(chrono::Utc::now());
        });
        Ok(())
    }

    /// 接收 BEP 消息（标准 BEP 帧格式）
    pub async fn recv_message(&self) -> Result<(MessageType, Bytes)> {
        let mut stream = self.stream.lock().await;

        // 读取 2 字节 header length
        let mut hdr_len_buf = [0u8; 2];
        stream.read_exact(&mut hdr_len_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("read failed: {}", e)))?;
        let hdr_len = u16::from_be_bytes(hdr_len_buf) as usize;

        // 读取 header
        let mut hdr_buf = vec![0u8; hdr_len];
        stream.read_exact(&mut hdr_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("read failed: {}", e)))?;

        // 读取 4 字节 message length
        let mut msg_len_buf = [0u8; 4];
        stream.read_exact(&mut msg_len_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("read failed: {}", e)))?;
        let msg_len = u32::from_be_bytes(msg_len_buf) as usize;

        // 读取 message
        let mut msg_buf = vec![0u8; msg_len];
        stream.read_exact(&mut msg_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("read failed: {}", e)))?;
        drop(stream);

        let bep_header = <bep_protocol::messages::Header as prost::Message>::decode(&hdr_buf[..])
            .map_err(|e| SyncthingError::protocol(format!("decode header failed: {}", e)))?;
        let header = MessageHeader::from_bep_header(&bep_header)
            .ok_or_else(|| SyncthingError::protocol(format!("unknown message type: {}", bep_header.r#type)))?;
        let payload = Bytes::copy_from_slice(&msg_buf);

        self.update_stats(|s| {
            s.messages_received += 1;
            s.bytes_received += (2 + hdr_len + 4 + msg_len) as u64;
            s.last_activity = Some(chrono::Utc::now());
        });
        Ok((header.message_type, payload))
    }

    pub async fn request_block(
        &self,
        folder: &str,
        name: &str,
        offset: i64,
        size: i32,
        hash: &[u8],
    ) -> Result<()> {
        let req = crate::protocol::RequestMessage {
            id: 0,
            folder: folder.to_string(),
            name: name.to_string(),
            offset,
            size,
            hash: hash.to_vec(),
        };
        let payload = serde_json::to_vec(&req)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        self.send_message(MessageType::Request, Bytes::from(payload)).await
    }

    pub async fn close(&self) -> Result<()> {
        info!("Closing iroh connection {}", self.id());
        self.set_state(ConnectionState::Disconnecting);
        let mut stream = self.stream.lock().await;
        let _ = stream.shutdown().await;
        drop(stream);
        self.set_state(ConnectionState::Disconnected);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::BEP_MAGIC;
    
    #[test]
    fn test_message_header_bep_roundtrip() {
        let header = MessageHeader {
            message_type: MessageType::Ping,
            message_id: 0,
            compressed: false,
        };

        let bep = header.to_bep_header();
        let decoded = MessageHeader::from_bep_header(&bep).unwrap();

        assert_eq!(decoded.message_type, MessageType::Ping);
        assert_eq!(decoded.compressed, false);
    }

    #[test]
    fn test_message_header_compression() {
        let header = MessageHeader {
            message_type: MessageType::Index,
            message_id: 0,
            compressed: true,
        };

        let bep = header.to_bep_header();
        assert_eq!(bep.compression, bep_protocol::messages::MessageCompression::Lz4 as i32);
        let decoded = MessageHeader::from_bep_header(&bep).unwrap();
        assert!(decoded.compressed);
    }
}
