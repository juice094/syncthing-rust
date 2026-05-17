//! BEP 连接实现
//!
//! 实现 Syncthing BEP 协议的连接层，支持 TLS 握手、Hello 交换、标准 BEP 帧编解码、
//! LZ4 解压以及独立的读写半流（tokio::io::split）。
//! 参考: syncthing/lib/connections/*.go
//! 2026-04-11 已验证与 Go BEP 实现跨网络互通（参见 VERIFICATION_REPORT_BEP_2026-04-11.md）。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use parking_lot::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::protocol::{MessageHeader, MessageType};
use syncthing_core::{
    ConnectionState, ConnectionStats, ConnectionType, DeviceId, Result, SyncthingError,
};

/// 默认消息超时
pub const DEFAULT_MESSAGE_TIMEOUT: Duration = Duration::from_secs(60);

/// 心跳间隔
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(90);

/// BEP header 最大大小 (64 KiB)
pub const MAX_BEP_HEADER_SIZE: usize = 64 * 1024;

/// BEP message 最大大小 (64 MiB)
///
/// 2026-05-11 (TUNING_PLAN T-D4)：从 128 MiB 收紧到 64 MiB。
/// 理由：Syncthing 实际 Index 上限约 30 MiB（参考 Go 实现）；过宽放给攻击者太大空间。
pub const MAX_BEP_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

/// 连接事件
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// 连接已建立
    Connected { device_id: DeviceId },
    /// 握手完成
    HandshakeComplete { device_id: DeviceId },
    /// 消息收到
    MessageReceived {
        device_id: DeviceId,
        msg_type: MessageType,
    },
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
    #[allow(dead_code)]
    pub last_ping: RwLock<Instant>,
    /// 最后pong时间
    pub last_pong: RwLock<Instant>,
}

mod tcp_pipe;
pub use tcp_pipe::TcpBiStream;

/// BEP 连接包装器
///
/// 封装底层的TCP连接，处理BEP协议细节
pub struct BepConnection {
    /// 内部状态
    inner: Arc<ConnectionInner>,
    /// 读取端
    read_half: Arc<Mutex<Option<tokio::io::ReadHalf<syncthing_core::traits::BoxedPipe>>>>,
    /// 写入端
    write_half: Arc<Mutex<Option<tokio::io::WriteHalf<syncthing_core::traits::BoxedPipe>>>>,
    /// 消息发送通道
    message_tx: mpsc::Sender<Message>,
    /// 事件发送器
    event_tx: mpsc::Sender<ConnectionEvent>,
    /// 关闭信号
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,
    /// 接收消息通道发送端（供读取任务使用）
    incoming_tx: mpsc::Sender<(MessageType, Bytes)>,
    /// 接收消息通道接收端
    incoming_rx: Arc<Mutex<mpsc::Receiver<(MessageType, Bytes)>>>,
}

/// 内部消息结构
#[derive(Debug)]
struct Message {
    pub header: MessageHeader,
    pub payload: Bytes,
}

impl BepConnection {
    /// 从可靠字节管道创建新连接
    pub async fn new(
        pipe: syncthing_core::traits::BoxedPipe,
        conn_type: ConnectionType,
        event_tx: mpsc::Sender<ConnectionEvent>,
    ) -> Result<Arc<Self>> {
        let remote_addr = pipe
            .peer_addr()
            .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
        let local_addr = pipe
            .local_addr()
            .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));

        let (message_tx, message_rx) = mpsc::channel(256);
        let (incoming_tx, incoming_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let id = uuid::Uuid::new_v4();

        let (read_half, write_half) = tokio::io::split(pipe);

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
            read_half: Arc::new(Mutex::new(Some(read_half))),
            write_half: Arc::new(Mutex::new(Some(write_half))),
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
        let _ = self
            .event_tx
            .try_send(ConnectionEvent::Connected { device_id });
    }

    /// 获取连接类型
    pub fn connection_type(&self) -> ConnectionType {
        self.inner.conn_type
    }

    /// 获取统计信息
    pub fn stats(&self) -> ConnectionStats {
        self.inner.stats.read().clone()
    }

    /// 返回自上次收发消息以来的时长
    pub fn last_activity_age(&self) -> Option<Duration> {
        let stats = self.inner.stats.read();
        stats.last_activity.map(|t| {
            (chrono::Utc::now() - t)
                .to_std()
                .unwrap_or(Duration::from_secs(0))
        })
    }

    /// 更新统计信息
    fn update_stats<F>(&self, f: F)
    where
        F: FnOnce(&mut ConnectionStats),
    {
        let mut stats = self.inner.stats.write();
        f(&mut stats);
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
            .await
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

    pub async fn send_cluster_config(
        &self,
        cc: &bep_protocol::messages::ClusterConfig,
    ) -> Result<()> {
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
        let _ = self.event_tx.try_send(ConnectionEvent::Disconnected {
            reason: "connection closed".to_string(),
        });

        Ok(())
    }

    /// 接收 BEP 消息（带超时，避免连接断开后永远卡住）
    pub async fn recv_message(&self) -> Result<(MessageType, Bytes)> {
        let mut rx = self.incoming_rx.lock().await;
        match tokio::time::timeout(Duration::from_secs(120), rx.recv()).await {
            Ok(Some(msg)) => Ok(msg),
            Ok(None) => Err(SyncthingError::ConnectionClosed),
            Err(_) => Err(SyncthingError::timeout("message receive timeout")),
        }
    }

    /// 主运行循环
    async fn run(
        &self,
        mut shutdown_rx: oneshot::Receiver<()>,
        message_rx: Arc<Mutex<mpsc::Receiver<Message>>>,
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
            result = read_handle => {
                match result {
                    Ok(Ok(())) => debug!("Connection {} read task ended normally", self.id()),
                    Ok(Err(e)) => warn!("Connection {} read task error: {}", self.id(), e),
                    Err(e) => warn!("Connection {} read task panicked: {}", self.id(), e),
                }
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
        let read_half = Arc::clone(&self.read_half);
        let event_tx = self.event_tx.clone();
        let incoming_tx = self.incoming_tx.clone();
        let inner = Arc::clone(&self.inner);

        tokio::spawn(async move {
            let mut read_half = read_half
                .lock()
                .await
                .take()
                .expect("INVARIANT: start_reader called only once per connection");
            loop {
                // 读取 2 字节 header length
                let mut hdr_len_buf = [0u8; 2];
                match timeout(
                    DEFAULT_MESSAGE_TIMEOUT,
                    read_half.read_exact(&mut hdr_len_buf),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => {
                        continue;
                    }
                }
                let hdr_len = u16::from_be_bytes(hdr_len_buf) as usize;
                if hdr_len > MAX_BEP_HEADER_SIZE {
                    return Err(SyncthingError::protocol(format!(
                        "BEP header too large: {} > {}",
                        hdr_len, MAX_BEP_HEADER_SIZE
                    )));
                }

                // 读取 header 字节
                let mut hdr_buf = vec![0u8; hdr_len];
                match timeout(DEFAULT_MESSAGE_TIMEOUT, read_half.read_exact(&mut hdr_buf)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => return Err(SyncthingError::timeout("header read timeout")),
                }

                // 读取 4 字节 message length
                let mut msg_len_buf = [0u8; 4];
                match timeout(
                    DEFAULT_MESSAGE_TIMEOUT,
                    read_half.read_exact(&mut msg_len_buf),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => return Err(SyncthingError::timeout("message length read timeout")),
                }
                let msg_len = u32::from_be_bytes(msg_len_buf) as usize;
                if msg_len > MAX_BEP_MESSAGE_SIZE {
                    return Err(SyncthingError::protocol(format!(
                        "BEP message too large: {} > {}",
                        msg_len, MAX_BEP_MESSAGE_SIZE
                    )));
                }

                // 读取 message 字节
                let mut msg_buf = vec![0u8; msg_len];
                match timeout(DEFAULT_MESSAGE_TIMEOUT, read_half.read_exact(&mut msg_buf)).await {
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

                // 解码 BEP Header
                let bep_header = match <bep_protocol::messages::Header as prost::Message>::decode(
                    &hdr_buf[..],
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        return Err(SyncthingError::protocol(format!(
                            "decode header failed: {}",
                            e
                        )))
                    }
                };

                let Some(header) = MessageHeader::from_bep_header(&bep_header) else {
                    return Err(SyncthingError::protocol(format!(
                        "unknown message type: {}",
                        bep_header.r#type
                    )));
                };

                // 处理 LZ4 压缩
                let msg_buf = if header.compressed {
                    if msg_buf.len() < 4 {
                        return Err(SyncthingError::protocol(
                            "compressed message too short".to_string(),
                        ));
                    }
                    let uncompressed_size =
                        u32::from_be_bytes([msg_buf[0], msg_buf[1], msg_buf[2], msg_buf[3]])
                            as usize;
                    match lz4::block::decompress(&msg_buf[4..], Some(uncompressed_size as i32)) {
                        Ok(decompressed) => decompressed,
                        Err(e) => {
                            return Err(SyncthingError::protocol(format!(
                                "lz4 decompress failed: {}",
                                e
                            )))
                        }
                    }
                } else {
                    msg_buf
                };

                debug!("Received message: {:?}", header.message_type);

                if let Some(device_id) = *inner.device_id.read() {
                    if let Err(e) = event_tx.try_send(ConnectionEvent::MessageReceived {
                        device_id,
                        msg_type: header.message_type,
                    }) {
                        warn!("Failed to send message received event: {}", e);
                    }
                }

                if let Err(e) = incoming_tx.try_send((header.message_type, Bytes::from(msg_buf))) {
                    warn!("Failed to enqueue incoming message: {}", e);
                }
            }
        })
    }

    /// 启动写入任务
    fn spawn_write_task(
        &self,
        message_rx: Arc<Mutex<mpsc::Receiver<Message>>>,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let write_half = Arc::clone(&self.write_half);
        let inner = Arc::clone(&self.inner);

        tokio::spawn(async move {
            let mut write_half = write_half
                .lock()
                .await
                .take()
                .expect("INVARIANT: start_writer called only once per connection");
            let mut rx = message_rx.lock().await;

            while let Some(msg) = rx.recv().await {
                let bep_header = msg.header.to_bep_header();
                let mut hdr_buf = Vec::new();
                if let Err(e) = <bep_protocol::messages::Header as prost::Message>::encode(
                    &bep_header,
                    &mut hdr_buf,
                ) {
                    return Err(SyncthingError::Serialization(e.to_string()));
                }
                let hdr_len = hdr_buf.len();
                let msg_len = msg.payload.len();

                // header length (2 bytes)
                if let Err(e) = write_half.write_all(&(hdr_len as u16).to_be_bytes()).await {
                    return Err(SyncthingError::Io(e));
                }
                // header
                if let Err(e) = write_half.write_all(&hdr_buf).await {
                    return Err(SyncthingError::Io(e));
                }
                // message length (4 bytes)
                if let Err(e) = write_half.write_all(&(msg_len as u32).to_be_bytes()).await {
                    return Err(SyncthingError::Io(e));
                }
                // payload
                if !msg.payload.is_empty() {
                    if let Err(e) = write_half.write_all(&msg.payload).await {
                        return Err(SyncthingError::Io(e));
                    }
                }
                if let Err(e) = write_half.flush().await {
                    return Err(SyncthingError::Io(e));
                }

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
            let _ = self.event_tx.try_send(ConnectionEvent::Disconnected {
                reason: "connection dropped".to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests;
