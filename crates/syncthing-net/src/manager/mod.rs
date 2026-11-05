//! Connection manager.
//!
//! Manages all active connections and handles connection establishment,
//! teardown, and reconnection.
//! (reference: syncthing/lib/connections/service.go, registry.go).
//! Simultaneous-dial race resolution: see "docs/design/ADR-20261105-connection-race.md".

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Weak};

use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::{mpsc, RwLock as TokioRwLock};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{info, warn};

use syncthing_core::{DeviceId, Identity, SyncthingError};

use crate::connection::{BepConnection, ConnectionEvent};
use crate::dialer::ParallelDialer;
use crate::netmon::NetChangeEvent;
use crate::tcp_transport::TcpTransport;
use crate::tls::SyncthingTlsConfig;
use crate::transport::TransportRegistry;

pub mod config;
pub mod dialer;
pub mod entry;
pub mod events;
pub mod handle;
pub mod handshake;
pub mod registry;
pub mod stats;

pub use config::ConnectionManagerConfig;
pub use handle::ConnectionManagerHandle;
pub use stats::{ManagerStats, ReconnectScheduler};

pub(crate) use entry::{ConnectionEntry, PendingConnection};

/// Callback invoked when a new device connects.
pub type ConnectedCallback = Arc<dyn Fn(DeviceId) + Send + Sync>;
/// Callback invoked when a device disconnects.
pub type DisconnectedCallback = Arc<dyn Fn(DeviceId, String) + Send + Sync>;

/// Connection manager.
pub struct ConnectionManager {
    /// Configuration.
    pub(crate) config: ConnectionManagerConfig,
    /// Local device identity (abstraction layer decoupled from the concrete
    /// cryptography scheme). The `Arc` is kept to retain the trait object
    /// lifetime; `local_device_id` is cached separately.
    // TODO: dynamic identity switching (currently single identity per process)
    #[allow(dead_code)]
    pub(crate) identity: Arc<dyn Identity>,
    /// Local device ID (cached from `identity` to avoid virtual calls).
    pub(crate) local_device_id: DeviceId,
    /// Active connection pool (device_id -> conn_id -> connection).
    pub(crate) connections: DashMap<DeviceId, DashMap<uuid::Uuid, ConnectionEntry>>,
    /// Index by connection ID (conn_id -> device_id).
    pub(crate) conn_id_index: DashMap<uuid::Uuid, DeviceId>,
    /// In-flight TLS/BEP handshakes (resolves the simultaneous-dial race before Hello).
    pub(crate) pending_handshakes: DashMap<DeviceId, handshake::HandshakeState>,
    /// Devices with a connection attempt in progress.
    pub(crate) pending_connections: TokioRwLock<HashMap<DeviceId, PendingConnection>>,
    /// Retry counters (independent of pending_connections; cleared on success).
    pub(crate) retry_counts: TokioRwLock<HashMap<DeviceId, u32>>,
    /// Device address mapping.
    pub(crate) device_addresses: DashMap<DeviceId, Vec<SocketAddr>>,
    /// Device relay URL mapping.
    pub(crate) device_relay_urls: DashMap<DeviceId, Vec<String>>,
    /// Event sender.
    pub(crate) event_tx: mpsc::Sender<ConnectionEvent>,
    /// Event receiver.
    pub(crate) event_rx: RwLock<Option<mpsc::Receiver<ConnectionEvent>>>,
    /// Running flag.
    pub(crate) running: RwLock<bool>,
    /// Maintenance task handle.
    pub(crate) maintenance_handle: RwLock<Option<JoinHandle<()>>>,
    /// Network monitor task handle.
    pub(crate) netmon_handle: RwLock<Option<JoinHandle<()>>>,
    /// Connected callback.
    pub(crate) on_connected: RwLock<Option<ConnectedCallback>>,
    /// Disconnected callback.
    pub(crate) on_disconnected: RwLock<Option<DisconnectedCallback>>,
    /// Self-referencing weak pointer.
    pub(crate) self_weak: RwLock<Option<Weak<ConnectionManager>>>,
    /// Parallel dialer.
    pub(crate) parallel_dialer: Arc<ParallelDialer>,
    /// TLS configuration (used by transport handshakes; Phase 2 will let the
    /// Transport trait manage its own).
    pub(crate) tls_config: Arc<SyncthingTlsConfig>,
    /// Transport registry (Phase 2: pluggable multi-transport).
    pub(crate) transport_registry: RwLock<Option<Arc<TransportRegistry>>>,
    /// Actual bound listen address.
    pub(crate) listen_addr: RwLock<Option<SocketAddr>>,
}

impl ConnectionManager {
    /// Returns the initialized `self_weak`.
    pub(crate) fn self_weak(&self) -> syncthing_core::Result<Weak<ConnectionManager>> {
        self.self_weak.read().clone().ok_or_else(|| {
            syncthing_core::SyncthingError::internal("ConnectionManager self_weak not initialized")
        })
    }

    /// Creates a new connection manager, returning the manager and its handle.
    pub fn new(
        config: ConnectionManagerConfig,
        identity: Arc<dyn Identity>,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> (Arc<Self>, ConnectionManagerHandle) {
        let local_device_id = identity.device_id();
        let (event_tx, event_rx) = mpsc::channel(256);

        crate::connection::set_default_heartbeat_interval(config.heartbeat_interval);

        let parallel_dialer = Arc::new(ParallelDialer::with_tcp_connector(
            local_device_id,
            "syncthing-rust".to_string(),
        ));

        let manager = Arc::new_cyclic(|weak| Self {
            config,
            identity,
            local_device_id,
            connections: DashMap::new(),
            conn_id_index: DashMap::new(),
            pending_handshakes: DashMap::new(),
            pending_connections: TokioRwLock::new(HashMap::new()),
            retry_counts: TokioRwLock::new(HashMap::new()),
            device_addresses: DashMap::new(),
            device_relay_urls: DashMap::new(),
            event_tx,
            event_rx: RwLock::new(Some(event_rx)),
            running: RwLock::new(false),
            maintenance_handle: RwLock::new(None),
            netmon_handle: RwLock::new(None),
            on_connected: RwLock::new(None),
            on_disconnected: RwLock::new(None),
            self_weak: RwLock::new(Some(weak.clone())),
            parallel_dialer,
            tls_config,
            transport_registry: RwLock::new(None),
            listen_addr: RwLock::new(None),
        });

        let handle = ConnectionManagerHandle {
            inner: Arc::clone(&manager),
        };

        (manager, handle)
    }

    /// Sets the callback invoked when a new device connects.
    pub fn on_connected<F>(&self, callback: F)
    where
        F: Fn(DeviceId) + Send + Sync + 'static,
    {
        *self.on_connected.write() = Some(Arc::new(callback));
    }

    /// Sets the callback invoked when a device disconnects.
    pub fn on_disconnected<F>(&self, callback: F)
    where
        F: Fn(DeviceId, String) + Send + Sync + 'static,
    {
        *self.on_disconnected.write() = Some(Arc::new(callback));
    }

    /// Sets the transport registry (must be called before `start()`).
    pub fn set_transport_registry(&self, registry: Arc<TransportRegistry>) {
        for scheme in registry.schemes() {
            if let Some(transport) = registry.get(scheme) {
                let connector = Arc::new(
                    crate::transport::bep_adapter::TransportBepConnector::new(transport),
                );
                self.parallel_dialer.register_connector(scheme, connector);
            }
        }
        if let Some(transport) = registry.default_transport() {
            let connector = Arc::new(crate::transport::bep_adapter::TransportBepConnector::new(
                transport,
            ));
            self.parallel_dialer.set_connector(connector);
        }
        *self.transport_registry.write() = Some(registry);
        let schemes = self
            .transport_registry
            .read()
            .as_ref()
            .map(|r| r.schemes())
            .unwrap_or_default();
        info!("Transport registry set with schemes: {:?}", schemes);
    }

    /// Starts the connection manager and returns the bound listen address.
    ///
    /// # Errors
    /// Returns an error if already running, not properly initialized, or no
    /// transport can bind the listen address.
    pub async fn start(&self) -> syncthing_core::Result<SocketAddr> {
        if *self.running.read() {
            return Err(SyncthingError::config("connection manager already running"));
        }

        info!("Starting connection manager...");

        let self_weak =
            self.self_weak.read().clone().ok_or_else(|| {
                SyncthingError::config("connection manager not properly initialized")
            })?;
        let handle = ConnectionManagerHandle {
            inner: self_weak
                .upgrade()
                .ok_or_else(|| SyncthingError::config("connection manager dropped"))?,
        };

        // Phase 2: prefer the TransportRegistry, fall back to the legacy TcpTransport.
        let default_transport = {
            let registry_guard = self.transport_registry.read();
            registry_guard.as_ref().and_then(|r| r.default_transport())
        };

        let listen_addr = if let Some(transport) = default_transport {
            match crate::transport::bep_adapter::BepTransportListener::start(
                transport.clone(),
                &self.config.listen_addr.to_string(),
                handle.clone(),
                self.local_device_id,
                "syncthing-rust".to_string(),
                Arc::clone(&self.tls_config),
            )
            .await
            {
                Ok(addr) => addr,
                Err(e) if self.config.listen_addr.port() != 0 => {
                    warn!(
                        "Transport listener failed to bind to {}, trying random port: {}",
                        self.config.listen_addr, e
                    );
                    let fallback_addr = "0.0.0.0:0".to_string();
                    crate::transport::bep_adapter::BepTransportListener::start(
                        transport,
                        &fallback_addr,
                        handle,
                        self.local_device_id,
                        "syncthing-rust".to_string(),
                        Arc::clone(&self.tls_config),
                    )
                    .await?
                }
                Err(e) => return Err(e),
            }
        } else {
            let mut tcp_transport = TcpTransport::new(
                self.config.listen_addr,
                handle.clone(),
                self.local_device_id,
                "syncthing-rust".to_string(),
                Arc::clone(&self.tls_config),
            );

            match tcp_transport.start().await {
                Ok(addr) => addr,
                Err(e) if self.config.listen_addr.port() != 0 => {
                    warn!(
                        "Failed to bind TCP listener to {}, trying random port: {}",
                        self.config.listen_addr, e
                    );
                    let fallback_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
                    let mut tcp_transport_fallback = TcpTransport::new(
                        fallback_addr,
                        handle,
                        self.local_device_id,
                        "syncthing-rust".to_string(),
                        Arc::clone(&self.tls_config),
                    );
                    tcp_transport_fallback.start().await?
                }
                Err(e) => return Err(e),
            }
        };

        *self.running.write() = true;

        self.spawn_event_handler();

        self.spawn_maintenance_task();

        *self.listen_addr.write() = Some(listen_addr);

        info!("Connection manager started on {}", listen_addr);

        Ok(listen_addr)
    }

    /// Stops the connection manager and disconnects all connections.
    pub async fn stop(&self) -> syncthing_core::Result<()> {
        info!("Stopping connection manager...");

        *self.running.write() = false;

        if let Some(handle) = self.netmon_handle.write().take() {
            handle.abort();
        }

        if let Some(handle) = self.maintenance_handle.write().take() {
            handle.abort();
        }

        self.disconnect_all("manager shutting down").await;

        info!("Connection manager stopped");
        Ok(())
    }

    /// Returns the first live connection to the device, if any.
    pub(crate) fn get_connection(&self, device_id: &DeviceId) -> Option<Arc<BepConnection>> {
        self.connections.get(device_id).and_then(|nested| {
            nested
                .iter()
                .find(|e| e.value().conn.is_alive())
                .map(|e| Arc::clone(&e.value().conn))
        })
    }

    /// Returns the connection with the given ID, if any.
    pub(crate) fn get_connection_by_id(&self, conn_id: &uuid::Uuid) -> Option<Arc<BepConnection>> {
        self.conn_id_index.get(conn_id).and_then(|device_id| {
            self.connections
                .get(&*device_id)
                .and_then(|nested| nested.get(conn_id).map(|e| Arc::clone(&e.conn)))
        })
    }

    /// Returns whether the device currently has a live connection.
    pub fn is_connected(&self, device_id: &DeviceId) -> bool {
        self.get_connection(device_id).is_some()
    }

    /// Returns the total number of currently active connections.
    pub fn active_connection_count(&self) -> usize {
        let mut count = 0usize;
        for device_entry in self.connections.iter() {
            for conn_entry in device_entry.value().iter() {
                if conn_entry.value().conn.is_alive() {
                    count += 1;
                }
            }
        }
        count
    }

    /// Returns whether a new connection can be accepted (enforced at `max_connections`).
    pub fn can_accept_connection(&self) -> bool {
        let current = self.active_connection_count();
        current < self.config.max_connections
    }

    /// Returns all devices with at least one live connection.
    pub fn connected_devices(&self) -> Vec<DeviceId> {
        self.connections
            .iter()
            .filter(|entry| entry.value().iter().any(|e| e.value().conn.is_alive()))
            .map(|entry| *entry.key())
            .collect()
    }

    /// Starts the network monitor task reacting to interface changes.
    pub fn start_netmon(&self, mut rx: mpsc::Receiver<NetChangeEvent>) {
        let weak = match self.self_weak() {
            Ok(w) => w,
            Err(e) => {
                warn!("Failed to get self_weak, aborting netmon task: {}", e);
                return;
            }
        };
        let handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Some(manager) = weak.upgrade() {
                    manager.handle_net_change(event).await;
                } else {
                    break;
                }
            }
        });
        *self.netmon_handle.write() = Some(handle);
    }

    /// Handles a network change: drops stale connections and redials known devices.
    async fn handle_net_change(&self, _event: NetChangeEvent) {
        info!("Network change detected, cleaning up stale connections and rebind dialing");

        self.cleanup_stale_connections().await;

        let devices: Vec<DeviceId> = self
            .device_addresses
            .iter()
            .map(|entry| *entry.key())
            .collect();

        for device_id in devices {
            if !self.is_connected(&device_id) {
                let addresses = self
                    .device_addresses
                    .get(&device_id)
                    .map(|e| e.clone())
                    .unwrap_or_default();
                let relay_urls = self
                    .device_relay_urls
                    .get(&device_id)
                    .map(|e| e.clone())
                    .unwrap_or_default();
                if !addresses.is_empty() || !relay_urls.is_empty() {
                    if let Err(e) = self
                        .connect_to_with_relay(device_id, addresses, relay_urls)
                        .await
                    {
                        warn!("Rebind redial to {} failed: {}", device_id, e);
                    }
                }
            }
        }
    }

    /// Returns whether a disconnect reason warrants an automatic reconnect.
    #[allow(clippy::unused_self)]
    pub(crate) fn should_reconnect(&self, _device_id: &DeviceId, reason: &str) -> bool {
        !reason.contains("manual disconnect")
            && !reason.contains("invalid device ID")
            && !reason.contains("unauthorized")
            && !reason.contains("paused by user")
    }

    /// Schedules an automatic reconnect with backoff for the device.
    pub(crate) async fn schedule_reconnect(&self, device_id: DeviceId) {
        let addresses = self
            .device_addresses
            .get(&device_id)
            .map(|e| e.clone())
            .unwrap_or_default();
        let relay_urls = self
            .device_relay_urls
            .get(&device_id)
            .map(|e| e.clone())
            .unwrap_or_default();

        if addresses.is_empty() && relay_urls.is_empty() {
            warn!(
                "No addresses available for device {}, skipping reconnect",
                device_id
            );
            return;
        }

        let retry_count = {
            let mut retries = self.retry_counts.write().await;
            let count = retries
                .entry(device_id)
                .and_modify(|c| *c += 1)
                .or_insert(1);
            *count
        };

        let backoff = self.config.retry_config.backoff_duration(retry_count);

        info!(
            "Scheduling reconnect to {} in {:?} (retry_count={})",
            device_id, backoff, retry_count
        );

        let weak = match self.self_weak() {
            Ok(w) => w,
            Err(e) => {
                warn!("Failed to get self_weak, aborting reconnect: {}", e);
                return;
            }
        };
        tokio::spawn(async move {
            sleep(backoff).await;

            if let Some(manager) = weak.upgrade() {
                manager.pending_connections.write().await.remove(&device_id);
                if let Err(e) = manager
                    .connect_to_with_relay(device_id, addresses, relay_urls)
                    .await
                {
                    warn!("Scheduled reconnect to {} failed: {}", device_id, e);
                }
            }
        });
    }

    /// Disconnects connections that are dead or idle beyond the configured timeout.
    pub(crate) async fn cleanup_stale_connections(&self) {
        let stale_threshold = self.config.connection_timeout;
        let mut stale_conns: Vec<uuid::Uuid> = Vec::new();

        for device_entry in self.connections.iter() {
            for conn_entry in device_entry.value().iter() {
                let entry = conn_entry.value();
                if !entry.conn.is_alive() || entry.is_stale(stale_threshold) {
                    stale_conns.push(*conn_entry.key());
                }
            }
        }

        for conn_id in stale_conns {
            if let Err(e) = self
                .disconnect_connection(&conn_id, "stale connection")
                .await
            {
                warn!("Error disconnecting stale connection {}: {}", conn_id, e);
            }
        }
    }

    /// Returns a snapshot of aggregated manager statistics.
    pub fn stats(&self) -> ManagerStats {
        let mut active_connections: usize = 0;
        let mut total_bytes_sent: u64 = 0;
        let mut total_bytes_received: u64 = 0;

        for entry in self.connections.iter() {
            for conn_entry in entry.value().iter() {
                let ce = conn_entry.value();
                if ce.conn.is_alive() {
                    active_connections += 1;
                }
                let stats = ce.conn.stats();
                total_bytes_sent += stats.bytes_sent;
                total_bytes_received += stats.bytes_received;
            }
        }

        ManagerStats {
            active_connections,
            connected_devices: self.connected_devices().len(),
            pending_connections: 0,
            total_bytes_sent,
            total_bytes_received,
        }
    }
}

#[cfg(test)]
mod tests;
