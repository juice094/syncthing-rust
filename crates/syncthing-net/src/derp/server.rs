//! DERP server.
//!
//! Minimal relay server responsible for:
//! 1. Accepting client TCP/WebSocket connections
//! 2. Registering clients by device_id
//! 3. Forwarding SendPacket as RecvPacket to the target device_id
//! 4. Keepalive detection and timeout cleanup
//!
//! Camouflage: can run over WebSocket/HTTPS so traffic looks like normal web.
//! Chaining: supports Relay A -> Relay B cascading.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

use crate::derp::protocol::{Frame, PROTOCOL_VERSION};

/// DERP server configuration.
#[derive(Debug, Clone)]
pub struct DerpServerConfig {
    /// Bind address.
    pub bind_addr: SocketAddr,
    /// Client timeout (disconnect when no keepalive).
    pub client_timeout: Duration,
    /// Whether to enable WebSocket upgrade (HTTPS camouflage).
    pub websocket_upgrade: bool,
}

impl Default for DerpServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 3478),
            client_timeout: Duration::from_secs(120),
            websocket_upgrade: false,
        }
    }
}

/// Handle to a connected client.
#[derive(Clone)]
struct ClientHandle {
    #[allow(dead_code)]
    device_id: DeviceId,
    addr: SocketAddr,
    last_activity: Arc<RwLock<Instant>>,
    /// Send channel (server -> client write task).
    tx: tokio::sync::mpsc::Sender<Frame>,
}

/// DERP relay server.
pub struct DerpServer {
    config: DerpServerConfig,
    listener: Option<Arc<TcpListener>>,
    /// device_id -> ClientHandle.
    clients: Arc<DashMap<DeviceId, ClientHandle>>,
    /// addr -> device_id (used for cleanup on disconnect).
    addr_index: Arc<DashMap<SocketAddr, DeviceId>>,
}

impl DerpServer {
    /// Creates a new server (call `bind` before `run`).
    pub fn new(config: DerpServerConfig) -> Self {
        Self {
            config,
            listener: None,
            clients: Arc::new(DashMap::new()),
            addr_index: Arc::new(DashMap::new()),
        }
    }

    /// Binds the listen address and returns the actual bound address.
    ///
    /// # Errors
    /// Returns an error if the bind or local address query fails.
    pub async fn bind(&mut self) -> Result<SocketAddr> {
        let listener = TcpListener::bind(self.config.bind_addr)
            .await
            .map_err(|e| SyncthingError::connection(format!("DERP server bind failed: {}", e)))?;
        let addr = listener.local_addr().map_err(|e| {
            SyncthingError::connection(format!("DERP server local_addr failed: {}", e))
        })?;
        self.listener = Some(Arc::new(listener));
        Ok(addr)
    }

    /// Returns the bound local address, if bound.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }

    /// Runs the server (blocks until shutdown). `bind()` must be called first.
    ///
    /// # Errors
    /// Returns an error if the server was not bound.
    pub async fn run(&self) -> Result<()> {
        let listener =
            Arc::clone(self.listener.as_ref().ok_or_else(|| {
                SyncthingError::config("DERP server not bound, call bind() first")
            })?);

        info!(
            "DERP server listening on {} (websocket_upgrade={})",
            self.local_addr().unwrap_or(self.config.bind_addr),
            self.config.websocket_upgrade
        );

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
                        let _ = client.tx.try_send(Frame::ClosePeer { target: device_id });
                    }
                }
            }
        });

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let clients = Arc::clone(&self.clients);
                    let addr_index = Arc::clone(&self.addr_index);
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_client(stream, addr, clients, addr_index).await
                        {
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

        let mut len_buf = [0u8; 4];
        read_half
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| SyncthingError::connection(format!("ClientInfo read failed: {}", e)))?;
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        read_half.read_exact(&mut payload_buf).await.map_err(|e| {
            SyncthingError::connection(format!("ClientInfo payload read failed: {}", e))
        })?;

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

        let server_info = Frame::ServerInfo {
            version: PROTOCOL_VERSION,
        };
        let encoded = server_info.encode();
        write_half
            .write_all(&encoded)
            .await
            .map_err(|e| SyncthingError::connection(format!("ServerInfo send failed: {}", e)))?;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Frame>(1024);
        let last_activity = Arc::new(RwLock::new(Instant::now()));

        let client = ClientHandle {
            device_id,
            addr,
            last_activity: Arc::clone(&last_activity),
            tx: tx.clone(),
        };

        if let Some(old) = clients.insert(device_id, client) {
            warn!(
                "DERP client {} reconnected from {}, dropping old connection from {}",
                device_id, addr, old.addr
            );
            addr_index.remove(&old.addr);
        }
        addr_index.insert(addr, device_id);

        info!("DERP client {} connected from {}", device_id, addr);

        let write_task = async {
            while let Some(frame) = rx.recv().await {
                let encoded = frame.encode();
                if write_half.write_all(&encoded).await.is_err() {
                    break;
                }
            }
        };

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
                            if target_client.tx.try_send(recv).is_err() {
                                warn!("DERP failed to forward packet to {}", target);
                            } else {
                                debug!("DERP forwarded packet from {} to {}", device_id, target);
                            }
                        } else {
                            debug!(
                                "DERP target {} not connected, dropping packet from {}",
                                target, device_id
                            );
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

        if let Some((_, removed)) = clients.remove(&device_id) {
            if removed.addr == addr {
                addr_index.remove(&addr);
                info!("DERP client {} disconnected from {}", device_id, addr);
            } else {
                // Superseded by a newer connection; do not remove it.
                clients.insert(device_id, removed);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    #[test]
    fn test_server_config_default() {
        let config = DerpServerConfig::default();
        assert_eq!(config.bind_addr.port(), 3478);
        assert!(!config.websocket_upgrade);
    }

    /// 端到端测试：两个客户端通过 DERP 服务器互相转发数据包
    #[tokio::test]
    async fn test_derp_relay_packet() {
        let config = DerpServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        };
        let mut server = DerpServer::new(config);
        let addr = server.bind().await.unwrap();

        // 启动服务器
        tokio::spawn(async move {
            let _ = server.run().await;
        });

        // 等待服务器启动
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let device_a = DeviceId::from_bytes(&[1u8; 32]).unwrap();
        let device_b = DeviceId::from_bytes(&[2u8; 32]).unwrap();

        // 客户端 A 连接
        let mut stream_a = TcpStream::connect(addr).await.unwrap();
        let client_info_a = Frame::ClientInfo {
            device_id: device_a,
            version: PROTOCOL_VERSION,
        };
        stream_a.write_all(&client_info_a.encode()).await.unwrap();

        // 读取 ServerInfo A
        let mut len_buf = [0u8; 4];
        stream_a.read_exact(&mut len_buf).await.unwrap();
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        stream_a.read_exact(&mut payload_buf).await.unwrap();
        let mut combined = bytes::BytesMut::from(&len_buf[..]);
        combined.extend_from_slice(&payload_buf);
        let (frame, _) = Frame::decode(&mut combined).unwrap().unwrap();
        assert!(matches!(frame, Frame::ServerInfo { .. }));

        // 客户端 B 连接
        let mut stream_b = TcpStream::connect(addr).await.unwrap();
        let client_info_b = Frame::ClientInfo {
            device_id: device_b,
            version: PROTOCOL_VERSION,
        };
        stream_b.write_all(&client_info_b.encode()).await.unwrap();

        // 读取 ServerInfo B
        stream_b.read_exact(&mut len_buf).await.unwrap();
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        stream_b.read_exact(&mut payload_buf).await.unwrap();
        let mut combined = bytes::BytesMut::from(&len_buf[..]);
        combined.extend_from_slice(&payload_buf);
        let (frame, _) = Frame::decode(&mut combined).unwrap().unwrap();
        assert!(matches!(frame, Frame::ServerInfo { .. }));

        // A 发送数据包到 B
        let send_packet = Frame::SendPacket {
            target: device_b,
            payload: b"hello from A".to_vec(),
        };
        stream_a.write_all(&send_packet.encode()).await.unwrap();

        // B 读取 RecvPacket
        stream_b.read_exact(&mut len_buf).await.unwrap();
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        stream_b.read_exact(&mut payload_buf).await.unwrap();
        let mut combined = bytes::BytesMut::from(&len_buf[..]);
        combined.extend_from_slice(&payload_buf);
        let (frame, _) = Frame::decode(&mut combined).unwrap().unwrap();

        match frame {
            Frame::RecvPacket { from, payload } => {
                assert_eq!(from, device_a);
                assert_eq!(payload, b"hello from A");
            }
            other => {
                panic!("expected RecvPacket, got {:?}", other);
            }
        }
    }
}
