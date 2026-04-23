//! DERP 客户端
//!
//! 负责连接到 DERP 中继服务器，注册本地 device_id，
//! 并发送/接收被中继的数据包。
//!
//! 未来扩展：
//! - 自动选择最优 DERP 服务器（基于 RTT）
//! - DERP 服务器链式转发（Client → Relay A → Relay B → Peer）
//! - 与 `TransportRegistry` 集成，将 DERP 作为 `Transport` 实现

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, timeout};
use tracing::{debug, info, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

use crate::derp::protocol::{Frame, PROTOCOL_VERSION};

/// DERP 客户端配置
#[derive(Debug, Clone)]
pub struct DerpClientConfig {
    /// DERP 服务器地址
    pub server_addr: SocketAddr,
    /// 本地 device_id
    pub device_id: DeviceId,
    /// 心跳间隔
    pub keep_alive_interval: Duration,
    /// 连接超时
    pub connect_timeout: Duration,
}

impl Default for DerpClientConfig {
    fn default() -> Self {
        Self {
            server_addr: "127.0.0.1:3478".parse().unwrap(),
            device_id: DeviceId::default(),
            keep_alive_interval: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

/// DERP 客户端状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerpClientState {
    Disconnected,
    Connecting,
    Connected,
}

/// Type alias for the inbound packet channel.
type PacketRx = mpsc::UnboundedReceiver<(DeviceId, Vec<u8>)>;

/// DERP 客户端
///
/// 通过 TCP 连接到 DERP 服务器，维护长连接并转发数据包。
pub struct DerpClient {
    config: DerpClientConfig,
    state: Arc<Mutex<DerpClientState>>,
    /// 出站数据包发送通道（应用层 → DERP 客户端）
    packet_tx: mpsc::UnboundedSender<(DeviceId, Vec<u8>)>,
    /// 入站数据包接收通道（DERP 客户端 → 应用层）
    packet_rx: Arc<Mutex<PacketRx>>,
}

impl DerpClient {
    /// 创建新的 DERP 客户端（不立即连接）
    pub fn new(config: DerpClientConfig) -> Self {
        let (packet_tx, packet_rx) = mpsc::unbounded_channel();
        Self {
            config,
            state: Arc::new(Mutex::new(DerpClientState::Disconnected)),
            packet_tx,
            packet_rx: Arc::new(Mutex::new(packet_rx)),
        }
    }

    /// 获取出站数据包发送通道
    pub fn packet_sender(&self) -> mpsc::UnboundedSender<(DeviceId, Vec<u8>)> {
        self.packet_tx.clone()
    }

    /// 连接到 DERP 服务器并启动事件循环
    pub async fn connect(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if *state != DerpClientState::Disconnected {
            return Err(SyncthingError::connection("DERP client already connected or connecting"));
        }
        *state = DerpClientState::Connecting;
        drop(state);

        info!("Connecting to DERP server at {}", self.config.server_addr);

        let stream = timeout(
            self.config.connect_timeout,
            TcpStream::connect(self.config.server_addr),
        )
        .await
        .map_err(|_| SyncthingError::timeout("DERP connect timeout"))?
        .map_err(|e| SyncthingError::connection(format!("DERP TCP connect failed: {}", e)))?;

        stream
            .set_nodelay(true)
            .map_err(|e| SyncthingError::connection(format!("set_nodelay failed: {}", e)))?;

        let (mut read_half, mut write_half) = stream.into_split();

        // 发送 ClientInfo
        let client_info = Frame::ClientInfo {
            device_id: self.config.device_id,
            version: PROTOCOL_VERSION,
        };
        let encoded = client_info.encode();
        write_half
            .write_all(&encoded)
            .await
            .map_err(|e| SyncthingError::connection(format!("DERP ClientInfo send failed: {}", e)))?;

        // 读取 ServerInfo
        let mut len_buf = [0u8; 4];
        read_half
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("DERP ServerInfo read failed: {}", e)))?;
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        read_half
            .read_exact(&mut payload_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("DERP ServerInfo payload read failed: {}", e)))?;

        let mut combined = bytes::BytesMut::from(&len_buf[..]);
        combined.extend_from_slice(&payload_buf);
        let (frame, _) = crate::derp::protocol::Frame::decode(&mut combined)
            .map_err(|e| SyncthingError::protocol(format!("DERP ServerInfo decode failed: {}", e)))?
            .ok_or_else(|| SyncthingError::protocol("DERP ServerInfo frame incomplete"))?;

        match frame {
            Frame::ServerInfo { version } => {
                info!("DERP server handshake OK, protocol version {}", version);
            }
            other => {
                return Err(SyncthingError::protocol(format!(
                    "expected ServerInfo, got {:?}",
                    other
                )));
            }
        }

        *self.state.lock().await = DerpClientState::Connected;

        // 启动读写任务
        let packet_rx = Arc::clone(&self.packet_rx);
        let packet_tx = self.packet_tx.clone();
        let keep_alive_interval = self.config.keep_alive_interval;
        let _device_id = self.config.device_id;

        tokio::spawn(async move {
            let packet_rx = Arc::clone(&packet_rx);
            let read_task = async {
                let mut len_buf = [0u8; 4];
                loop {
                    if read_half.read_exact(&mut len_buf).await.is_err() {
                        break;
                    }
                    let payload_len = u32::from_be_bytes(len_buf) as usize;
                    if payload_len > crate::derp::protocol::MAX_FRAME_SIZE as usize {
                        warn!("DERP received oversized frame");
                        break;
                    }
                    let mut payload_buf = vec![0u8; payload_len];
                    if read_half.read_exact(&mut payload_buf).await.is_err() {
                        break;
                    }
                    let mut combined = bytes::BytesMut::from(&len_buf[..]);
                    combined.extend_from_slice(&payload_buf);
                    match crate::derp::protocol::Frame::decode(&mut combined) {
                        Ok(Some((Frame::RecvPacket { from, payload }, _))) => {
                            // 将收到的数据包转发给应用层
                            if let Err(e) = packet_tx.send((from, payload)) {
                                warn!("DERP failed to forward packet to app layer: {}", e);
                            }
                        }
                        Ok(Some((Frame::KeepAlive, _))) => {
                            debug!("DERP keepalive received");
                        }
                        Ok(Some(other)) => {
                            debug!("DERP received frame: {:?}", other);
                        }
                        Ok(None) => {
                            warn!("DERP incomplete frame");
                        }
                        Err(e) => {
                            warn!("DERP frame decode error: {}", e);
                        }
                    }
                }
            };

            let write_task = async {
                let mut keep_alive = interval(keep_alive_interval);
                let _packet_rx = packet_rx.lock().await;
                loop {
                    tokio::select! {
                        _ = keep_alive.tick() => {
                            let keepalive = Frame::KeepAlive.encode();
                            if write_half.write_all(&keepalive).await.is_err() {
                                break;
                            }
                        }
                        // 实际生产代码：从 packet_rx 接收数据并发送
                        // 当前 stub：仅保活
                    }
                }
            };

            tokio::select! {
                _ = read_task => {},
                _ = write_task => {},
            }

            info!("DERP client disconnected from server");
        });

        Ok(())
    }

    pub async fn state(&self) -> DerpClientState {
        *self.state.lock().await
    }
}
