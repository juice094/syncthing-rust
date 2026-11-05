use std::net::SocketAddr;
use std::sync::Arc;

use syncthing_core::DeviceId;

use crate::connection::BepConnection;

use super::{handshake::HandshakeGuard, ConnectionManager, ManagerStats};

/// Connection manager handle (for sharing across threads).
#[derive(Clone)]
pub struct ConnectionManagerHandle {
    pub(crate) inner: Arc<ConnectionManager>,
}

impl ConnectionManagerHandle {
    /// Registers a pending handshake after TLS, before the BEP Hello.
    ///
    /// Returns `Err` when this connection should lose the device ID race; the
    /// caller must close the socket immediately.
    /// See "docs/design/ADR-20261105-connection-race.md".
    ///
    /// # Errors
    /// Returns an error when the handshake loses the race.
    pub fn begin_handshake(
        &self,
        device_id: DeviceId,
        conn_type: syncthing_core::ConnectionType,
    ) -> syncthing_core::Result<HandshakeGuard> {
        self.inner.begin_handshake(device_id, conn_type)
    }

    /// Registers a new connection (called by the transport layer).
    ///
    /// # Errors
    /// Returns an error if registration fails in the manager.
    pub async fn register_connection(
        &self,
        device_id: DeviceId,
        conn: Arc<BepConnection>,
    ) -> syncthing_core::Result<()> {
        self.inner.register_connection(device_id, conn).await
    }

    /// Registers an incoming connection (device ID must already be set).
    ///
    /// # Errors
    /// Returns an error if the connection has no device ID.
    pub async fn register_incoming(&self, conn: Arc<BepConnection>) -> syncthing_core::Result<()> {
        self.inner.register_incoming(conn).await
    }

    /// Returns the first live connection to the given device.
    pub fn get_connection(&self, device_id: &DeviceId) -> Option<Arc<BepConnection>> {
        self.inner.get_connection(device_id)
    }

    /// Returns all devices with at least one live connection.
    pub fn connected_devices(&self) -> Vec<DeviceId> {
        self.inner.connected_devices()
    }

    /// Disconnects all connections to the device.
    ///
    /// # Errors
    /// Returns an error if teardown fails in the manager.
    pub async fn disconnect(
        &self,
        device_id: &DeviceId,
        reason: &str,
    ) -> syncthing_core::Result<()> {
        self.inner.disconnect(device_id, reason).await
    }

    /// Disconnects the connection with the given ID.
    ///
    /// # Errors
    /// Returns an error if teardown fails in the manager.
    pub async fn disconnect_connection(
        &self,
        conn_id: &uuid::Uuid,
        reason: &str,
    ) -> syncthing_core::Result<()> {
        self.inner.disconnect_connection(conn_id, reason).await
    }

    /// Returns the connection with the given ID.
    pub fn get_connection_by_id(&self, conn_id: &uuid::Uuid) -> Option<Arc<BepConnection>> {
        self.inner.get_connection_by_id(conn_id)
    }

    /// Returns the actual bound listen address, if started.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        *self.inner.listen_addr.read()
    }

    /// Connects to the device using the given addresses.
    ///
    /// # Errors
    /// Returns an error if dialing fails for all candidates.
    pub async fn connect_to(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
    ) -> syncthing_core::Result<()> {
        self.inner
            .connect_to_with_relay(device_id, addresses, vec![])
            .await
    }

    /// Connects to the device using the given addresses, falling back to relays.
    ///
    /// # Errors
    /// Returns an error if dialing fails for all candidates.
    pub async fn connect_to_with_relay(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
    ) -> syncthing_core::Result<()> {
        self.inner
            .connect_to_with_relay(device_id, addresses, relay_urls)
            .await
    }

    /// Force-reconnects a device by ID, using the internal address pool.
    ///
    /// # Errors
    /// Returns an error if redialing fails.
    pub async fn reconnect_device_by_id(&self, device_id: DeviceId) -> syncthing_core::Result<()> {
        let addresses = self
            .inner
            .device_addresses
            .get(&device_id)
            .map(|e| e.clone())
            .unwrap_or_default();
        let relay_urls = self
            .inner
            .device_relay_urls
            .get(&device_id)
            .map(|e| e.clone())
            .unwrap_or_default();
        self.reconnect_device(device_id, addresses, relay_urls)
            .await
    }

    /// Force-reconnects to a device: disconnects existing connections and redials.
    ///
    /// Unlike `connect_to`, this runs the full disconnect → clear pending →
    /// dial flow even when the device is connected, ensuring `on_connected`
    /// fires again.
    ///
    /// # Errors
    /// Returns an error if redialing fails.
    pub async fn reconnect_device(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
    ) -> syncthing_core::Result<()> {
        self.disconnect(&device_id, "reconnect").await.ok();
        {
            let mut pending = self.inner.pending_connections.write().await;
            pending.remove(&device_id);
        }
        self.connect_to_with_relay(device_id, addresses, relay_urls)
            .await
    }

    /// Updates the device's address pool (called by the discovery layer).
    pub fn update_addresses(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
    ) {
        if !addresses.is_empty() {
            self.inner.device_addresses.insert(device_id, addresses);
        }
        if !relay_urls.is_empty() {
            self.inner.device_relay_urls.insert(device_id, relay_urls);
        }
    }

    /// Stops the connection manager.
    ///
    /// # Errors
    /// Returns an error if teardown fails in the manager.
    pub async fn stop(&self) -> syncthing_core::Result<()> {
        self.inner.stop().await
    }

    /// Returns whether a new connection can be accepted (enforced at `max_connections`).
    pub fn can_accept_connection(&self) -> bool {
        self.inner.can_accept_connection()
    }

    /// Returns the total number of currently active connections.
    pub fn active_connection_count(&self) -> usize {
        self.inner.active_connection_count()
    }

    /// Returns a snapshot of aggregated manager statistics.
    pub fn stats(&self) -> ManagerStats {
        self.inner.stats()
    }
}

#[async_trait::async_trait]
impl syncthing_core::traits::ConnectionManager for ConnectionManagerHandle {
    fn connected_devices(&self) -> Vec<syncthing_core::DeviceId> {
        self.connected_devices()
    }

    async fn disconnect(
        &self,
        device_id: &syncthing_core::DeviceId,
        reason: &str,
    ) -> syncthing_core::Result<()> {
        self.disconnect(device_id, reason).await
    }

    fn connection_stats(&self) -> syncthing_core::traits::AggregateConnectionStats {
        let stats = self.stats();
        syncthing_core::traits::AggregateConnectionStats {
            total_bytes_sent: stats.total_bytes_sent,
            total_bytes_received: stats.total_bytes_received,
        }
    }

    fn has_connection(&self, device_id: &syncthing_core::DeviceId) -> bool {
        self.get_connection(device_id).is_some()
    }

    fn get_connection_info(
        &self,
        device_id: &syncthing_core::DeviceId,
    ) -> Option<syncthing_core::traits::ConnectionInfo> {
        self.get_connection(device_id)
            .map(|conn| syncthing_core::traits::ConnectionInfo {
                remote_addr: conn.remote_addr().to_string(),
                is_alive: conn.is_alive(),
            })
    }

    async fn connect_to(
        &self,
        device_id: &syncthing_core::DeviceId,
        addresses: Vec<std::net::SocketAddr>,
    ) -> syncthing_core::Result<()> {
        self.connect_to(*device_id, addresses).await
    }
}
