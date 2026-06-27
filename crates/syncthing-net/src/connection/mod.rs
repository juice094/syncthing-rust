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
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, error, info, warn};

use crate::protocol::{MessageHeader, MessageType};
use syncthing_core::{
    ConnectionState, ConnectionStats, ConnectionType, DeviceId, Result, SyncthingError,
};

/// 默认消息超时
pub const DEFAULT_MESSAGE_TIMEOUT: Duration = Duration::from_secs(60);

/// 默认心跳间隔（未通过配置覆盖时）
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(90);

/// 全局默认心跳间隔，由 `ConnectionManager::new` 从配置注入。
/// 允许外部通过配置覆盖默认 90s，同时仍按路径类型保留上限。
static DEFAULT_HEARTBEAT_INTERVAL: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();

/// 设置全局默认心跳间隔。
pub fn set_default_heartbeat_interval(interval: Duration) {
    let _ = DEFAULT_HEARTBEAT_INTERVAL.set(interval);
}

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
pub(super) struct ConnectionInner {
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
    /// 本连接的心跳间隔（按路径类型调优）
    pub heartbeat_interval: Duration,
}

mod tcp_pipe;
pub use tcp_pipe::TcpBiStream;
mod io;

/// BEP 连接包装器
///
/// 封装底层的TCP连接，处理BEP协议细节
pub struct BepConnection {
    /// 内部状态
    pub(super) inner: Arc<ConnectionInner>,
    /// 读取端
    pub(super) read_half:
        Arc<Mutex<Option<tokio::io::ReadHalf<syncthing_core::traits::BoxedPipe>>>>,
    /// 写入端
    pub(super) write_half:
        Arc<Mutex<Option<tokio::io::WriteHalf<syncthing_core::traits::BoxedPipe>>>>,
    /// 消息发送通道
    message_tx: mpsc::Sender<Message>,
    /// 事件发送器
    pub(super) event_tx: mpsc::Sender<ConnectionEvent>,
    /// 关闭信号
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,
    /// 接收消息通道发送端（供读取任务使用）
    pub(super) incoming_tx: mpsc::Sender<(MessageType, Bytes)>,
    /// 接收消息通道接收端
    incoming_rx: Arc<Mutex<mpsc::Receiver<(MessageType, Bytes)>>>,
}

/// 内部消息结构
#[derive(Debug)]
pub(super) struct Message {
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
        let transport_type = pipe.transport_type();

        // 按路径类型选择心跳间隔：中间 relay/proxy/websocket/derp 使用更短心跳防止被掐断。
        // 全局默认心跳可由配置覆盖，relay 路径最多不超过 30s。
        let base_interval = DEFAULT_HEARTBEAT_INTERVAL
            .get()
            .copied()
            .unwrap_or(HEARTBEAT_INTERVAL);
        let heartbeat_interval = match transport_type {
            syncthing_core::TransportType::Tcp => base_interval,
            syncthing_core::TransportType::Memory => base_interval.min(Duration::from_secs(10)),
            _ => base_interval.min(Duration::from_secs(30)),
        };

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
                heartbeat_interval,
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
        // 使用 From<&Index> 避免 .clone() + .into() 双重分配
        let wire: bep_protocol::messages::Index = index.into();
        let payload = bep_protocol::messages::encode_message(&wire)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        self.send_message(MessageType::Index, payload).await
    }

    pub async fn send_index_update(&self, update: &syncthing_core::IndexUpdate) -> Result<()> {
        let wire: bep_protocol::messages::IndexUpdate = update.into();
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
