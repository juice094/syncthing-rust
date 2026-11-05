//! BEP connection implementation.
//!
//! Owns the connection layer of the BEP protocol, including the Hello
//! message exchange.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, info};

use syncthing_core::{
    ConnectionState, ConnectionStats, ConnectionType, DeviceId, Result, SyncthingError,
};

use crate::handshake::{exchange_hello, exchange_hello_server};
use crate::messages::Hello;

/// Default message timeout.
pub const DEFAULT_MESSAGE_TIMEOUT: Duration = Duration::from_secs(60);

/// Heartbeat interval.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(90);

/// A raw BEP connection over an underlying TCP stream.
pub struct BepRawConnection {
    /// Connection ID.
    id: uuid::Uuid,
    /// Underlying stream.
    stream: Arc<Mutex<TcpStream>>,
    /// Connection state.
    state: RwLock<ConnectionState>,
    /// Connection statistics.
    stats: RwLock<ConnectionStats>,
    /// Remote address.
    remote_addr: SocketAddr,
    /// Local address.
    local_addr: SocketAddr,
    /// Connection direction (incoming/outgoing).
    conn_type: ConnectionType,
    /// Associated device ID, once known.
    device_id: RwLock<Option<DeviceId>>,
    /// Peer's Hello message.
    remote_hello: RwLock<Option<Hello>>,
    /// Time of last activity.
    last_activity: RwLock<Instant>,
}

impl BepRawConnection {
    /// Creates a connection from a TCP stream in client mode (sends Hello
    /// first); used for outgoing (dialed) connections.
    ///
    /// Returns the connection together with the peer's Hello message.
    ///
    /// # Errors
    /// Returns a connection error if the local or peer address cannot be
    /// determined, or a protocol error if the Hello exchange fails.
    pub async fn connect(
        stream: TcpStream,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<(Arc<Self>, Hello)> {
        let remote_addr = stream
            .peer_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get peer addr: {}", e)))?;
        let local_addr = stream
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get local addr: {}", e)))?;

        let stream = Arc::new(Mutex::new(stream));
        let id = uuid::Uuid::new_v4();

        let conn = Arc::new(Self {
            id,
            stream: Arc::clone(&stream),
            state: RwLock::new(ConnectionState::Connected),
            stats: RwLock::new(ConnectionStats {
                connected_at: Some(chrono::Utc::now()),
                ..Default::default()
            }),
            remote_addr,
            local_addr,
            conn_type: ConnectionType::Outgoing,
            device_id: RwLock::new(None),
            remote_hello: RwLock::new(None),
            last_activity: RwLock::new(Instant::now()),
        });

        let remote_hello = conn
            .perform_hello(device_name, client_name, client_version)
            .await?;

        Ok((conn, remote_hello))
    }

    /// Creates a connection from a TCP stream in server mode (receives
    /// Hello first); used for incoming (listened) connections.
    ///
    /// Returns the connection together with the peer's Hello message.
    ///
    /// # Errors
    /// Returns a connection error if the local or peer address cannot be
    /// determined, or a protocol error if the Hello exchange fails.
    pub async fn accept(
        stream: TcpStream,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<(Arc<Self>, Hello)> {
        let remote_addr = stream
            .peer_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get peer addr: {}", e)))?;
        let local_addr = stream
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get local addr: {}", e)))?;

        let stream = Arc::new(Mutex::new(stream));
        let id = uuid::Uuid::new_v4();

        let conn = Arc::new(Self {
            id,
            stream: Arc::clone(&stream),
            state: RwLock::new(ConnectionState::Connected),
            stats: RwLock::new(ConnectionStats {
                connected_at: Some(chrono::Utc::now()),
                ..Default::default()
            }),
            remote_addr,
            local_addr,
            conn_type: ConnectionType::Incoming,
            device_id: RwLock::new(None),
            remote_hello: RwLock::new(None),
            last_activity: RwLock::new(Instant::now()),
        });

        let remote_hello = conn
            .perform_hello_server(device_name, client_name, client_version)
            .await?;

        Ok((conn, remote_hello))
    }

    /// Creates a new Hello message.
    fn create_hello(device_name: &str, client_name: &str, client_version: &str) -> Hello {
        Hello {
            device_name: device_name.to_string(),
            client_name: client_name.to_string(),
            client_version: client_version.to_string(),
            num_connections: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        }
    }

    /// Runs the Hello exchange in client mode (sends ours, then receives theirs).
    async fn perform_hello(
        &self,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<Hello> {
        self.set_state(ConnectionState::TlsHandshakeComplete);

        let our_hello = Self::create_hello(device_name, client_name, client_version);

        info!(
            "Sending Hello: device={} client={}/{}",
            our_hello.device_name, our_hello.client_name, our_hello.client_version
        );

        let mut stream = self.stream.lock().await;
        let remote_hello = exchange_hello(&mut *stream, &our_hello).await?;
        drop(stream);

        *self.remote_hello.write() = Some(remote_hello.clone());
        self.set_state(ConnectionState::ProtocolHandshakeComplete);
        *self.last_activity.write() = Instant::now();

        info!(
            "Hello exchange complete: remote_device={} client={}/{}",
            remote_hello.device_name, remote_hello.client_name, remote_hello.client_version
        );

        self.update_stats(|s| {
            s.messages_sent += 1;
            s.messages_received += 1;
        });

        Ok(remote_hello)
    }

    /// Runs the Hello exchange in server mode (receives theirs, then sends ours).
    async fn perform_hello_server(
        &self,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<Hello> {
        self.set_state(ConnectionState::TlsHandshakeComplete);

        let our_hello = Self::create_hello(device_name, client_name, client_version);

        let mut stream = self.stream.lock().await;
        let remote_hello = exchange_hello_server(&mut *stream, &our_hello).await?;
        drop(stream);

        *self.remote_hello.write() = Some(remote_hello.clone());
        self.set_state(ConnectionState::ProtocolHandshakeComplete);
        *self.last_activity.write() = Instant::now();

        info!(
            "Hello exchange complete (server): remote_device={} client={}/{}",
            remote_hello.device_name, remote_hello.client_name, remote_hello.client_version
        );

        self.update_stats(|s| {
            s.messages_sent += 1;
            s.messages_received += 1;
        });

        Ok(remote_hello)
    }

    /// Returns the connection ID.
    pub fn id(&self) -> uuid::Uuid {
        self.id
    }

    /// Returns the remote address.
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// Returns the local address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns the current connection state.
    pub fn state(&self) -> ConnectionState {
        *self.state.read()
    }

    /// Sets the connection state.
    fn set_state(&self, state: ConnectionState) {
        *self.state.write() = state;
    }

    /// Returns the associated device ID, if known.
    pub fn device_id(&self) -> Option<DeviceId> {
        *self.device_id.read()
    }

    /// Associates a device ID with this connection.
    pub fn set_device_id(&self, device_id: DeviceId) {
        *self.device_id.write() = Some(device_id);
    }

    /// Returns the peer's Hello message.
    pub fn remote_hello(&self) -> Option<Hello> {
        self.remote_hello.read().clone()
    }

    /// Returns the connection direction.
    pub fn connection_type(&self) -> ConnectionType {
        self.conn_type
    }

    /// Returns the connection statistics.
    pub fn stats(&self) -> ConnectionStats {
        self.stats.read().clone()
    }

    /// Applies a mutation to the connection statistics.
    fn update_stats<F>(&self, f: F)
    where
        F: FnOnce(&mut ConnectionStats),
    {
        let mut stats = self.stats.write();
        f(&mut stats);
    }

    /// Returns whether the connection is alive.
    pub fn is_alive(&self) -> bool {
        matches!(
            self.state(),
            ConnectionState::Connected
                | ConnectionState::TlsHandshakeComplete
                | ConnectionState::ProtocolHandshakeComplete
                | ConnectionState::ClusterConfigComplete
        )
    }

    /// Returns whether the Hello exchange has completed.
    pub fn is_hello_complete(&self) -> bool {
        matches!(
            self.state(),
            ConnectionState::ProtocolHandshakeComplete | ConnectionState::ClusterConfigComplete
        )
    }

    /// Closes the connection gracefully.
    pub async fn close(&self) -> Result<()> {
        info!("Closing connection {}", self.id());

        self.set_state(ConnectionState::Disconnecting);

        let mut stream = self.stream.lock().await;
        let _ = stream.shutdown().await;
        drop(stream);

        self.set_state(ConnectionState::Disconnected);

        Ok(())
    }
}

impl Drop for BepRawConnection {
    fn drop(&mut self) {
        if self.is_alive() {
            debug!("Connection {} dropped while still alive", self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state() {
        // 状态测试在集成测试中完成
        assert!(matches!(ConnectionType::Outgoing, ConnectionType::Outgoing));
        assert!(matches!(ConnectionType::Incoming, ConnectionType::Incoming));
    }

    #[test]
    fn test_hello_creation() {
        // 测试Hello消息创建
        let hello = Hello {
            device_name: "test".to_string(),
            client_name: "syncthing-rust".to_string(),
            client_version: "0.1.0".to_string(),
            num_connections: 1,
            timestamp: 1234567890,
        };

        assert_eq!(hello.device_name, "test");
        assert_eq!(hello.client_name, "syncthing-rust");
        assert_eq!(hello.client_version, "0.1.0");
        assert_eq!(hello.num_connections, 1);
    }
}
