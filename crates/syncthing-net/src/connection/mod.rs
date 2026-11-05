//! BEP connection implementation.
//!
//! Implements the connection layer of the Syncthing BEP protocol: TLS handshake,
//! Hello exchange, standard BEP frame encoding/decoding, LZ4 decompression,
//! and independent read/write half-streams (tokio::io::split).
//! (reference: syncthing/lib/connections/*.go;
//! 2026-04-11 verified wire interoperability with the Go BEP implementation,
//! see VERIFICATION_REPORT_BEP_2026-04-11.md).
//! Heartbeat and size-limit tuning rationale: see "docs/design/ADR-20261105-connection-tuning.md".

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

/// Default message timeout.
pub const DEFAULT_MESSAGE_TIMEOUT: Duration = Duration::from_secs(60);

/// Default heartbeat interval (when not overridden by configuration).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(90);

/// Global default heartbeat interval, injected by `ConnectionManager::new`
/// from configuration. Allows overriding the 90 s default while keeping
/// per-path-type caps.
static DEFAULT_HEARTBEAT_INTERVAL: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();

/// Sets the global default heartbeat interval.
pub fn set_default_heartbeat_interval(interval: Duration) {
    let _ = DEFAULT_HEARTBEAT_INTERVAL.set(interval);
}

/// Maximum BEP header size (64 KiB).
pub const MAX_BEP_HEADER_SIZE: usize = 64 * 1024;

/// Maximum BEP message size (64 MiB).
///
/// Tightened from 128 MiB on 2026-05-11; rationale: see "docs/design/ADR-20261105-connection-tuning.md".
pub const MAX_BEP_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

/// Connection event.
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// Connection established.
    Connected { device_id: DeviceId },
    /// Handshake completed.
    HandshakeComplete { device_id: DeviceId },
    /// Message received.
    MessageReceived {
        device_id: DeviceId,
        msg_type: MessageType,
    },
    /// Connection closed.
    Disconnected { reason: String },
    /// Error occurred.
    Error { error: String },
}

/// Internal connection state.
pub(super) struct ConnectionInner {
    /// Connection ID.
    pub id: uuid::Uuid,
    /// Remote address.
    pub remote_addr: SocketAddr,
    /// Local address.
    pub local_addr: SocketAddr,
    /// Connection state.
    pub state: RwLock<ConnectionState>,
    /// Connection statistics.
    pub stats: RwLock<ConnectionStats>,
    /// Connection type.
    pub conn_type: ConnectionType,
    /// Associated device ID.
    pub device_id: RwLock<Option<DeviceId>>,
    /// Last ping time.
    #[allow(dead_code)]
    pub last_ping: RwLock<Instant>,
    /// Last pong time.
    pub last_pong: RwLock<Instant>,
    /// Per-connection heartbeat interval (tuned by path type).
    pub heartbeat_interval: Duration,
}

mod tcp_pipe;
pub use tcp_pipe::TcpBiStream;
mod io;

/// BEP connection wrapper.
///
/// Wraps the underlying byte pipe and handles BEP protocol details.
pub struct BepConnection {
    /// Internal state.
    pub(super) inner: Arc<ConnectionInner>,
    /// Read half.
    pub(super) read_half:
        Arc<Mutex<Option<tokio::io::ReadHalf<syncthing_core::traits::BoxedPipe>>>>,
    /// Write half.
    pub(super) write_half:
        Arc<Mutex<Option<tokio::io::WriteHalf<syncthing_core::traits::BoxedPipe>>>>,
    /// Outgoing message channel.
    message_tx: mpsc::Sender<Message>,
    /// Event sender.
    pub(super) event_tx: mpsc::Sender<ConnectionEvent>,
    /// Shutdown signal.
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,
    /// Incoming message channel sender (used by the read task).
    pub(super) incoming_tx: mpsc::Sender<(MessageType, Bytes)>,
    /// Incoming message channel receiver.
    incoming_rx: Arc<Mutex<mpsc::Receiver<(MessageType, Bytes)>>>,
}

/// Internal message structure.
#[derive(Debug)]
pub(super) struct Message {
    pub header: MessageHeader,
    pub payload: Bytes,
}

impl BepConnection {
    /// Creates a new connection from a reliable byte pipe.
    ///
    /// Spawns the connection run task before returning.
    ///
    /// # Errors
    /// Currently always returns `Ok`; the `Result` is reserved for future
    /// fallible setup steps.
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

        let conn_clone = Arc::clone(&conn);
        let message_rx = Arc::new(Mutex::new(message_rx));
        tokio::spawn(async move {
            if let Err(e) = conn_clone.run(shutdown_rx, message_rx).await {
                error!("Connection {} error: {}", id, e);
            }
        });

        Ok(conn)
    }

    /// Returns the connection ID.
    pub fn id(&self) -> uuid::Uuid {
        self.inner.id
    }

    /// Returns the remote address.
    pub fn remote_addr(&self) -> SocketAddr {
        self.inner.remote_addr
    }

    /// Returns the local address.
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }

    /// Returns the current connection state.
    pub fn state(&self) -> ConnectionState {
        *self.inner.state.read()
    }

    /// Sets the connection state.
    pub fn set_state(&self, state: ConnectionState) {
        *self.inner.state.write() = state;
    }

    /// Returns the associated device ID, if set.
    pub fn device_id(&self) -> Option<DeviceId> {
        *self.inner.device_id.read()
    }

    /// Associates a device ID with this connection and emits `Connected`.
    pub fn set_device_id(&self, device_id: DeviceId) {
        *self.inner.device_id.write() = Some(device_id);

        let _ = self
            .event_tx
            .try_send(ConnectionEvent::Connected { device_id });
    }

    /// Returns the connection type (incoming/outgoing).
    pub fn connection_type(&self) -> ConnectionType {
        self.inner.conn_type
    }

    /// Returns a snapshot of the connection statistics.
    pub fn stats(&self) -> ConnectionStats {
        self.inner.stats.read().clone()
    }

    /// Returns the time elapsed since the last send/receive activity.
    pub fn last_activity_age(&self) -> Option<Duration> {
        let stats = self.inner.stats.read();
        stats.last_activity.map(|t| {
            (chrono::Utc::now() - t)
                .to_std()
                .unwrap_or(Duration::from_secs(0))
        })
    }

    /// Updates the connection statistics.
    fn update_stats<F>(&self, f: F)
    where
        F: FnOnce(&mut ConnectionStats),
    {
        let mut stats = self.inner.stats.write();
        f(&mut stats);
    }

    /// Sends a message with the given type and payload.
    ///
    /// # Errors
    /// Returns `SyncthingError::ConnectionClosed` if the connection task has
    /// terminated.
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

    /// Sends a Ping (BEP keepalive; one-way, no Pong exists in BEP — receiving
    /// a Ping must not be answered).
    ///
    /// # Errors
    /// Returns `SyncthingError::ConnectionClosed` if the connection task has
    /// terminated.
    pub async fn send_ping(&self) -> Result<()> {
        self.send_message(MessageType::Ping, Bytes::new()).await
    }

    /// Sends a keepalive frame (legacy alias of `send_ping`; BEP has no Pong,
    /// so never use this to answer a Ping).
    ///
    /// # Errors
    /// Returns `SyncthingError::ConnectionClosed` if the connection task has
    /// terminated.
    pub async fn send_pong(&self) -> Result<()> {
        self.send_message(MessageType::Ping, Bytes::new()).await
    }

    /// Encodes and sends a ClusterConfig message.
    ///
    /// # Errors
    /// Returns an error if encoding fails or the connection task has terminated.
    pub async fn send_cluster_config(
        &self,
        cc: &bep_protocol::messages::ClusterConfig,
    ) -> Result<()> {
        let payload = bep_protocol::messages::encode_message(cc)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        self.send_message(MessageType::ClusterConfig, payload).await
    }

    /// Encodes and sends an Index message.
    ///
    /// # Errors
    /// Returns an error if encoding fails or the connection task has terminated.
    pub async fn send_index(&self, index: &syncthing_core::Index) -> Result<()> {
        let wire: bep_protocol::messages::Index = index.into();
        let payload = bep_protocol::messages::encode_message(&wire)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        self.send_message(MessageType::Index, payload).await
    }

    /// Encodes and sends an IndexUpdate message.
    ///
    /// # Errors
    /// Returns an error if encoding fails or the connection task has terminated.
    pub async fn send_index_update(&self, update: &syncthing_core::IndexUpdate) -> Result<()> {
        let wire: bep_protocol::messages::IndexUpdate = update.into();
        let payload = bep_protocol::messages::encode_message(&wire)
            .map_err(|e| SyncthingError::Serialization(e.to_string()))?;
        self.send_message(MessageType::IndexUpdate, payload).await
    }

    /// Closes the connection.
    pub async fn close(&self) -> Result<()> {
        info!("Closing connection {}", self.id());

        self.set_state(ConnectionState::Disconnecting);

        if let Some(tx) = self.shutdown_tx.write().take() {
            let _ = tx.send(());
        }

        self.set_state(ConnectionState::Disconnected);

        let _ = self.event_tx.try_send(ConnectionEvent::Disconnected {
            reason: "connection closed".to_string(),
        });

        Ok(())
    }

    /// Receives the next BEP message, with a timeout so a dead connection
    /// does not block forever.
    ///
    /// # Errors
    /// Returns `SyncthingError::ConnectionClosed` if the channel is closed, or
    /// a timeout error after 120 seconds without a message.
    pub async fn recv_message(&self) -> Result<(MessageType, Bytes)> {
        let mut rx = self.incoming_rx.lock().await;
        match tokio::time::timeout(Duration::from_secs(120), rx.recv()).await {
            Ok(Some(msg)) => Ok(msg),
            Ok(None) => Err(SyncthingError::ConnectionClosed),
            Err(_) => Err(SyncthingError::timeout("message receive timeout")),
        }
    }

    /// Main run loop coordinating the read, write, and heartbeat tasks.
    async fn run(
        &self,
        mut shutdown_rx: oneshot::Receiver<()>,
        message_rx: Arc<Mutex<mpsc::Receiver<Message>>>,
    ) -> Result<()> {
        let read_handle = self.spawn_read_task();

        let write_handle = self.spawn_write_task(message_rx);

        let heartbeat_handle = self.spawn_heartbeat_task();

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

    /// Returns whether the connection is in a live state.
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
        if self.is_alive() {
            let _ = self.event_tx.try_send(ConnectionEvent::Disconnected {
                reason: "connection dropped".to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests;
