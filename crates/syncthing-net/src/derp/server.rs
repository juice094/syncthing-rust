//! DERP 服务器
//!
//! 极简的中继服务器，负责：
//! 1. 接受客户端 TCP/WebSocket 连接
//! 2. 按 device_id 注册客户端
//! 3. 收到 SendPacket 时按 target device_id 转发 RecvPacket
//! 4. 心跳检测与超时清理
//!
//! 可伪装：通过 WebSocket over HTTPS 运行，流量看起来像普通网站。
//! 可链式：支持 Relay A → Relay B 的级联转发。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

use crate::derp::protocol::{Frame, PROTOCOL_VERSION};

/// DERP 服务器配置
#[derive(Debug, Clone)]
pub struct DerpServerConfig {
    /// 监听地址
    pub bind_addr: SocketAddr,
    /// 客户端超时（无心跳则断开）
    pub client_timeout: Duration,
    /// 是否启用 WebSocket 升级（伪装为 HTTPS）
    pub websocket_upgrade: bool,
}

impl Default for DerpServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:3478".parse().unwrap(),
            client_timeout: Duration::from_secs(120),
            websocket_upgrade: false,
        }
    }
}

/// 客户端连接句柄
#[derive(Clone)]
struct ClientHandle {
    #[allow(dead_code)]
    device_id: DeviceId,
    addr: SocketAddr,
    last_activity: Arc<RwLock<Instant>>,
    /// 发送通道（服务器 → 客户端写任务）
    tx: tokio::sync::mpsc::UnboundedSender<Frame>,
}

/// DERP 中继服务器
pub struct DerpServer {
    config: DerpServerConfig,
    /// device_id → ClientHandle
    clients: Arc<DashMap<DeviceId, ClientHandle>>,
    /// addr → device_id（用于断开时清理）
    addr_index: Arc<DashMap<SocketAddr, DeviceId>>,
}

impl DerpServer {
    pub fn new(config: DerpServerConfig) -> Self {
        Self {
            config,
            clients: Arc::new(DashMap::new()),
            addr_index: Arc::new(DashMap::new()),
        }
    }

    /// 启动服务器（阻塞直到关闭）
    pub async fn run(&self) -> Result<()> {
        let listener = TcpListener::bind(self.config.bind_addr).await.map_err(|e| {
            SyncthingError::connection(format!("DERP server bind failed: {}", e))
        })?;

        info!(
            "DERP server listening on {} (websocket_upgrade={})",
            self.config.bind_addr, self.config.websocket_upgrade
        );

        // 启动超时清理任务
        let clients = Arc::clone(&self.clients);
        let addr_index = Arc::clone(&self.addr_index);
        let timeout = self.config.client_timeout;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let now = Instant::now();
                let mut to_remove = Vec::new();
                for entry in clients.iter() {
                    let last = *entry.value().last_activity.read().await;
                    if now.duration_since(last) > timeout {
                        to_remove.push(*entry.key());
                    }
                }
                for device_id in to_remove {
                    if let Some((_, client)) = clients.remove(&device_id) {
                        warn!("DERP client {} timed out, disconnecting", device_id);
                        addr_index.remove(&client.addr);
                        let _ = client.tx.send(Frame::ClosePeer { target: device_id });
                    }
                }
            }
        });

        // 接受循环
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let clients = Arc::clone(&self.clients);
                    let addr_index = Arc::clone(&self.addr_index);
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_client(stream, addr, clients, addr_index).await {
                            warn!("DERP client {} handler error: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("DERP accept error: {}", e);
                }
            }
        }
    }

    async fn handle_client(
        stream: TcpStream,
        addr: SocketAddr,
        clients: Arc<DashMap<DeviceId, ClientHandle>>,
        addr_index: Arc<DashMap<SocketAddr, DeviceId>>,
    ) -> Result<()> {
        stream
            .set_nodelay(true)
            .map_err(|e| SyncthingError::connection(format!("set_nodelay failed: {}", e)))?;

        let (mut read_half, mut write_half) = stream.into_split();

        // 读取 ClientInfo
        let mut len_buf = [0u8; 4];
        read_half
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("ClientInfo read failed: {}", e)))?;
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        read_half
            .read_exact(&mut payload_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("ClientInfo payload read failed: {}", e)))?;

        let mut combined = bytes::BytesMut::from(&len_buf[..]);
        combined.extend_from_slice(&payload_buf);
        let (frame, _) = crate::derp::protocol::Frame::decode(&mut combined)
            .map_err(|e| SyncthingError::protocol(format!("ClientInfo decode failed: {}", e)))?
            .ok_or_else(|| SyncthingError::protocol("ClientInfo frame incomplete"))?;

        let device_id = match frame {
            Frame::ClientInfo { device_id, version } => {
                if version != PROTOCOL_VERSION {
                    return Err(SyncthingError::protocol(format!(
                        "protocol version mismatch: client={}, server={}",
                        version, PROTOCOL_VERSION
                    )));
                }
                debug!("DERP client {} registered from {}", device_id, addr);
                device_id
            }
            other => {
                return Err(SyncthingError::protocol(format!(
                    "expected ClientInfo, got {:?}",
                    other
                )));
            }
        };

        // 发送 ServerInfo
        let server_info = Frame::ServerInfo {
            version: PROTOCOL_VERSION,
        };
        let encoded = server_info.encode();
        write_half
            .write_all(&encoded)
            .await
            .map_err(|e| SyncthingError::connection(format!("ServerInfo send failed: {}", e)))?;

        // 创建客户端通道
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Frame>();
        let last_activity = Arc::new(RwLock::new(Instant::now()));

        let client = ClientHandle {
            device_id,
            addr,
            last_activity: Arc::clone(&last_activity),
            tx: tx.clone(),
        };

        // 注册客户端（覆盖旧连接）
        if let Some(old) = clients.insert(device_id, client) {
            warn!("DERP client {} reconnected from {}, dropping old connection from {}", device_id, addr, old.addr);
            addr_index.remove(&old.addr);
        }
        addr_index.insert(addr, device_id);

        info!("DERP client {} connected from {}", device_id, addr);

        // 写任务：从 rx 接收帧并发送到客户端
        let write_task = async {
            while let Some(frame) = rx.recv().await {
                let encoded = frame.encode();
                if write_half.write_all(&encoded).await.is_err() {
                    break;
                }
            }
        };

        // 读任务：读取客户端发来的帧并处理
        let read_task = async {
            let mut len_buf = [0u8; 4];
            loop {
                if read_half.read_exact(&mut len_buf).await.is_err() {
                    break;
                }
                let payload_len = u32::from_be_bytes(len_buf) as usize;
                if payload_len > crate::derp::protocol::MAX_FRAME_SIZE as usize {
                    warn!("DERP oversized frame from {}", addr);
                    break;
                }
                let mut payload_buf = vec![0u8; payload_len];
                if read_half.read_exact(&mut payload_buf).await.is_err() {
                    break;
                }

                // 更新活跃时间
                *last_activity.write().await = Instant::now();

                let mut combined = bytes::BytesMut::from(&len_buf[..]);
                combined.extend_from_slice(&payload_buf);
                match crate::derp::protocol::Frame::decode(&mut combined) {
                    Ok(Some((Frame::SendPacket { target, payload }, _))) => {
                        if let Some(target_client) = clients.get(&target) {
                            let recv = Frame::RecvPacket {
                                from: device_id,
                                payload,
                            };
                            if target_client.tx.send(recv).is_err() {
                                warn!("DERP failed to forward packet to {}", target);
                            } else {
                                debug!("DERP forwarded packet from {} to {}", device_id, target);
                            }
                        } else {
                            debug!("DERP target {} not connected, dropping packet from {}", target, device_id);
                        }
                    }
                    Ok(Some((Frame::KeepAlive, _))) => {
                        debug!("DERP keepalive from {}", device_id);
                    }
                    Ok(Some(other)) => {
                        debug!("DERP unexpected frame from {}: {:?}", device_id, other);
                    }
                    Ok(None) => {
                        warn!("DERP incomplete frame from {}", addr);
                    }
                    Err(e) => {
                        warn!("DERP frame decode error from {}: {}", addr, e);
                    }
                }
            }
        };

        tokio::select! {
            _ = write_task => {},
            _ = read_task => {},
        }

        // 清理
        if let Some((_, removed)) = clients.remove(&device_id) {
            if removed.addr == addr {
                addr_index.remove(&addr);
                info!("DERP client {} disconnected from {}", device_id, addr);
            } else {
                // 被更新的连接覆盖，不要清理新连接
                clients.insert(device_id, removed);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = DerpServerConfig::default();
        assert_eq!(config.bind_addr.port(), 3478);
        assert!(!config.websocket_upgrade);
    }
}
