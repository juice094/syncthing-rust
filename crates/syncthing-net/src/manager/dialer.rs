use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::oneshot;
use tracing::{debug, warn};

use syncthing_core::{ConnectionType, DeviceId};

use super::ConnectionManager;

impl ConnectionManager {
    /// Connects to the device (with relay fallback), spawning the dial task.
    ///
    /// # Errors
    /// This function currently always returns `Ok`; dialing failures are
    /// handled by the spawned task and the reconnect scheduler.
    pub(crate) async fn connect_to_with_relay(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
    ) -> syncthing_core::Result<()> {
        // SAFETY: SSRF protection — filter dangerous addresses from untrusted
        // sources (discovery/relay/config) before dialing.
        let safe_addresses: Vec<SocketAddr> = addresses
            .into_iter()
            .filter(|addr| {
                if let Err(e) = crate::tcp_transport::validate_outbound_addr(addr) {
                    warn!(
                        device = %device_id,
                        address = %addr,
                        error = %e,
                        "Filtering out unsafe address from device address pool"
                    );
                    false
                } else {
                    true
                }
            })
            .collect();

        // Persist addresses and relay URLs before the is_connected check, so an
        // existing direct connection does not prevent storing relay candidates.
        self.device_addresses
            .insert(device_id, safe_addresses.clone());
        if !relay_urls.is_empty() {
            self.device_relay_urls.insert(device_id, relay_urls.clone());
        }

        // Atomically check connected/pending state and insert into the pending
        // table in one write critical section, preventing parallel dial storms
        // from concurrent network events (netmon redial + scheduled retries).
        let cancel_rx = {
            let mut pending = self.pending_connections.write().await;

            if self.is_connected(&device_id) {
                debug!("Device {} is already connected", device_id);
                return Ok(());
            }

            if pending.contains_key(&device_id) {
                debug!("Connection to {} is already pending", device_id);
                return Ok(());
            }

            let retry_count = pending.get(&device_id).map(|p| p.retry_count).unwrap_or(0);

            let (cancel_tx, cancel_rx) = oneshot::channel();
            pending.insert(
                device_id,
                super::PendingConnection {
                    device_id,
                    addresses: safe_addresses.clone(),
                    relay_urls: relay_urls.clone(),
                    retry_count,
                    last_attempt: Some(std::time::Instant::now()),
                    _cancel_tx: Some(cancel_tx),
                },
            );
            cancel_rx
        };

        // Register the handshake race state before TLS ClientHello; see
        // "docs/design/ADR-20261105-connection-race.md".
        let handshake_guard = match self.begin_handshake(device_id, ConnectionType::Outgoing) {
            Ok(guard) => guard,
            Err(e) => {
                debug!(
                    "Outgoing handshake race lost for {} before ClientHello: {}",
                    device_id, e
                );
                self.pending_connections.write().await.remove(&device_id);
                return Ok(());
            }
        };

        self.spawn_connect_task(
            device_id,
            safe_addresses,
            relay_urls,
            cancel_rx,
            handshake_guard,
        );

        Ok(())
    }

    /// Spawns the background dial task.
    fn spawn_connect_task(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
        mut cancel_rx: oneshot::Receiver<()>,
        #[allow(dead_code)] handshake_guard: super::handshake::HandshakeGuard,
    ) {
        let parallel_dialer = Arc::clone(&self.parallel_dialer);
        let tls_config = Arc::clone(&self.tls_config);
        let local_device_id = self.local_device_id;
        let self_weak = match self.self_weak() {
            Ok(w) => w,
            Err(e) => {
                warn!("Failed to get self_weak, aborting connect task: {}", e);
                return;
            }
        };

        tokio::spawn(async move {
            let _handshake_guard = handshake_guard;

            tokio::select! {
                _ = &mut cancel_rx => {
                    debug!("Connection task for {} cancelled", device_id);
                    if let Some(manager) = self_weak.upgrade() {
                        manager.pending_connections.write().await.remove(&device_id);
                    }
                }
                result = parallel_dialer.dial(
                    device_id,
                    addresses,
                    relay_urls,
                    &tls_config,
                    &local_device_id,
                ) => {
                    match result {
                        Ok(conn) => {
                            if let Some(manager) = self_weak.upgrade() {
                                if let Err(e) = manager.register_connection(device_id, conn).await {
                                    warn!("Failed to register connection for {}: {}", device_id, e);
                                    manager.pending_connections.write().await.remove(&device_id);
                                    manager.schedule_reconnect(device_id).await;
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to dial {}: {}", device_id, e);
                            if let Some(manager) = self_weak.upgrade() {
                                manager.pending_connections.write().await.remove(&device_id);
                                manager.schedule_reconnect(device_id).await;
                            }
                        }
                    }
                }
            }
        });
    }
}
