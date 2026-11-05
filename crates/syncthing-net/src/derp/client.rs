//! DERP client.
//!
//! Connects to a DERP relay server, registers the local device_id, and
//! sends/receives relayed packets.
//!
//! Future extensions:
//! - Automatic selection of the best DERP server (by RTT)
//! - DERP server chaining (Client -> Relay A -> Relay B -> Peer)
//! - Integration with `TransportRegistry` as a `Transport` implementation

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

/// DERP client configuration.
#[derive(Debug, Clone)]
pub struct DerpClientConfig {
    /// DERP server address.
    pub server_addr: SocketAddr,
    /// Local device ID.
    pub device_id: DeviceId,
    /// Keepalive interval.
    pub keep_alive_interval: Duration,
    /// Connect timeout.
    pub connect_timeout: Duration,
}

impl Default for DerpClientConfig {
    fn default() -> Self {
        Self {
            server_addr: SocketAddr::from(([127, 0, 0, 1], 3478)),
            device_id: DeviceId::default(),
            keep_alive_interval: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

/// DERP client state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerpClientState {
    /// Not connected.
    Disconnected,
    /// Connection handshake in progress.
    Connecting,
    /// Connected and registered.
    Connected,
}

/// Type alias for the inbound packet channel.
type PacketRx = mpsc::Receiver<(DeviceId, Vec<u8>)>;

/// DERP client.
///
/// Connects to the DERP server over TCP and maintains a long-lived connection
/// for packet forwarding.
pub struct DerpClient {
    config: DerpClientConfig,
    state: Arc<Mutex<DerpClientState>>,
    /// Outbound packet channel (app layer -> DERP client).
    packet_tx: mpsc::Sender<(DeviceId, Vec<u8>)>,
    /// Inbound packet channel (DERP client -> app layer).
    packet_rx: Arc<Mutex<PacketRx>>,
}

impl DerpClient {
    /// Creates a new DERP client (does not connect yet).
    pub fn new(config: DerpClientConfig) -> Self {
        let (packet_tx, packet_rx) = mpsc::channel(256);
        Self {
            config,
            state: Arc::new(Mutex::new(DerpClientState::Disconnected)),
            packet_tx,
            packet_rx: Arc::new(Mutex::new(packet_rx)),
        }
    }

    /// Returns the outbound packet sender channel.
    pub fn packet_sender(&self) -> mpsc::Sender<(DeviceId, Vec<u8>)> {
        self.packet_tx.clone()
    }

    /// Connects to the DERP server and starts the event loop.
    ///
    /// # Errors
    /// Returns an error if already connecting/connected, the TCP connect
    /// fails or times out, or the handshake frame exchange fails.
    pub async fn connect(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if *state != DerpClientState::Disconnected {
            return Err(SyncthingError::connection(
                "DERP client already connected or connecting",
            ));
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

        let client_info = Frame::ClientInfo {
            device_id: self.config.device_id,
            version: PROTOCOL_VERSION,
        };
        let encoded = client_info.encode();
        write_half.write_all(&encoded).await.map_err(|e| {
            SyncthingError::connection(format!("DERP ClientInfo send failed: {}", e))
        })?;

        let mut len_buf = [0u8; 4];
        read_half.read_exact(&mut len_buf).await.map_err(|e| {
            SyncthingError::connection(format!("DERP ServerInfo read failed: {}", e))
        })?;
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        read_half.read_exact(&mut payload_buf).await.map_err(|e| {
            SyncthingError::connection(format!("DERP ServerInfo payload read failed: {}", e))
        })?;

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
                            if let Err(e) = packet_tx.send((from, payload)).await {
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
                        // Outbound packet forwarding (packet_rx -> wire) is not
                        // implemented yet; the write task only sends keepalives.
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

    /// Returns the current client state.
    pub async fn state(&self) -> DerpClientState {
        *self.state.lock().await
    }
}
