use std::sync::Arc;

use tracing::{debug, info, warn};

use syncthing_core::{ConnectionType, DeviceId, SyncthingError};

use crate::connection::{BepConnection, ConnectionEvent};

use super::{ConnectionEntry, ConnectionManager};

/// Internal action for `register_connection`: decided while holding the
/// DashMap lock, executed after it is released (all `.await` points happen
/// outside the lock; see "docs/design/ADR-20261105-connection-race.md").
enum RegisterAction {
    /// Replace the old connection (close it, then insert the new one).
    Replace,
    /// Reject the new connection (close it, keep the old one).
    RejectNew,
    /// Insert into the existing nested DashMap.
    InsertExisting,
    /// Create a new nested DashMap.
    CreateNew,
}

impl ConnectionManager {
    /// Registers a new connection, resolving any simultaneous-dial race.
    ///
    /// # Errors
    /// This function currently always returns `Ok`.
    pub(crate) async fn register_connection(
        &self,
        device_id: DeviceId,
        conn: Arc<BepConnection>,
    ) -> syncthing_core::Result<()> {
        debug!("Registering connection for device {}", device_id);

        let conn_id = conn.id();
        let new_conn_type = conn.connection_type();

        // Decide under the lock, act after releasing it: holding a DashMap
        // (parking_lot) lock across .await can deadlock sibling tasks and
        // exhaust tokio workers during connection storms.
        let (action, old_conn_to_close) = {
            if let Some(nested) = self.connections.get(&device_id) {
                if let Some(existing) = nested.iter().next() {
                    let old_conn_id = *existing.key();
                    let old_conn_type = existing.value().conn.connection_type();
                    let old_conn_arc = Arc::clone(&existing.value().conn);

                    let should_replace = if old_conn_type == new_conn_type {
                        false
                    } else {
                        let local_smaller = self.local_device_id.0 < device_id.0;
                        match (old_conn_type, new_conn_type) {
                            (ConnectionType::Outgoing, ConnectionType::Incoming) => local_smaller,
                            (ConnectionType::Incoming, ConnectionType::Outgoing) => !local_smaller,
                            _ => {
                                warn!(
                                    "Unexpected connection type combination in race resolution: old={:?}, new={:?}",
                                    old_conn_type, new_conn_type
                                );
                                false
                            }
                        }
                    };

                    if should_replace {
                        info!(
                            "Closing existing connection {} for device {} (new {} via race resolution)",
                            old_conn_id, device_id, conn_id
                        );
                        (RegisterAction::Replace, Some(old_conn_arc))
                    } else {
                        info!("Closing new connection {} for device {} (keeping existing {} via race resolution)",
                              conn_id, device_id, old_conn_id);
                        (RegisterAction::RejectNew, None)
                    }
                } else {
                    (RegisterAction::InsertExisting, None)
                }
            } else {
                (RegisterAction::CreateNew, None)
            }
        };

        if let Some(old_conn) = old_conn_to_close {
            old_conn.close().await.ok();
        }

        match action {
            RegisterAction::Replace => {
                if let Some(nested) = self.connections.get_mut(&device_id) {
                    nested.clear();
                    nested.insert(conn_id, ConnectionEntry::new(Arc::clone(&conn)));
                } else {
                    let nested = dashmap::DashMap::new();
                    nested.insert(conn_id, ConnectionEntry::new(Arc::clone(&conn)));
                    self.connections.insert(device_id, nested);
                }
            }
            RegisterAction::RejectNew => {
                conn.close().await.ok();
                return Ok(());
            }
            RegisterAction::InsertExisting => {
                if let Some(nested) = self.connections.get_mut(&device_id) {
                    nested.insert(conn_id, ConnectionEntry::new(Arc::clone(&conn)));
                } else {
                    let nested = dashmap::DashMap::new();
                    nested.insert(conn_id, ConnectionEntry::new(Arc::clone(&conn)));
                    self.connections.insert(device_id, nested);
                }
            }
            RegisterAction::CreateNew => {
                let nested = dashmap::DashMap::new();
                nested.insert(conn_id, ConnectionEntry::new(Arc::clone(&conn)));
                self.connections.insert(device_id, nested);
            }
        }

        self.conn_id_index.insert(conn_id, device_id);

        {
            let mut pending = self.pending_connections.write().await;
            if pending.remove(&device_id).is_some() {
                debug!(
                    "Cleared pending state for {} (connection established)",
                    device_id
                );
            }
        }
        {
            let mut retries = self.retry_counts.write().await;
            retries.remove(&device_id);
        }

        conn.set_device_id(device_id);

        self.pending_connections.write().await.remove(&device_id);

        info!(
            "Connection registered for device {} (conn_id: {}, type: {:?})",
            device_id, conn_id, new_conn_type
        );

        if let Some(callback) = self.on_connected.read().as_ref() {
            callback(device_id);
        }

        let _ = self
            .event_tx
            .send(ConnectionEvent::Connected { device_id })
            .await;

        Ok(())
    }

    /// Registers an incoming connection (registers directly when the device ID is known).
    ///
    /// # Errors
    /// Returns an error if the connection has no device ID.
    pub(crate) async fn register_incoming(
        &self,
        conn: Arc<BepConnection>,
    ) -> syncthing_core::Result<()> {
        debug!("Registering incoming connection {}", conn.id());

        if let Some(device_id) = conn.device_id() {
            self.register_connection(device_id, conn).await
        } else {
            warn!(
                "Incoming connection {} has no device_id, skipping registration",
                conn.id()
            );
            Err(SyncthingError::connection(
                "incoming connection missing device ID",
            ))
        }
    }

    /// Disconnects all connections to the device and schedules a reconnect when applicable.
    ///
    /// # Errors
    /// This function currently always returns `Ok`.
    pub(crate) async fn disconnect(
        &self,
        device_id: &DeviceId,
        reason: &str,
    ) -> syncthing_core::Result<()> {
        info!("Disconnecting device {}: {}", device_id, reason);

        if let Some((_, nested)) = self.connections.remove(device_id) {
            for entry in nested {
                let (conn_id, e) = entry;
                e.conn.close().await.ok();
                self.conn_id_index.remove(&conn_id);
            }

            if let Some(callback) = self.on_disconnected.read().as_ref() {
                callback(*device_id, reason.to_string());
            }
        }

        if self.should_reconnect(device_id, reason) {
            self.schedule_reconnect(*device_id).await;
        }

        Ok(())
    }

    /// Disconnects the connection with the given ID.
    ///
    /// # Errors
    /// This function currently always returns `Ok`.
    pub(crate) async fn disconnect_connection(
        &self,
        conn_id: &uuid::Uuid,
        reason: &str,
    ) -> syncthing_core::Result<()> {
        let Some((_, device_id)) = self.conn_id_index.remove(conn_id) else {
            return Ok(());
        };

        let device_has_other_conns = if let Some(nested) = self.connections.get_mut(&device_id) {
            nested.remove(conn_id);
            !nested.is_empty()
        } else {
            false
        };

        if !device_has_other_conns {
            self.connections.remove(&device_id);
            if self.should_reconnect(&device_id, reason) {
                self.schedule_reconnect(device_id).await;
            }
            if let Some(callback) = self.on_disconnected.read().as_ref() {
                callback(device_id, reason.to_string());
            }
        }

        Ok(())
    }

    /// Disconnects all connections with the given reason.
    pub(crate) async fn disconnect_all(&self, reason: &str) {
        let devices: Vec<DeviceId> = self.connections.iter().map(|e| *e.key()).collect();

        for device_id in devices {
            if let Err(e) = self.disconnect(&device_id, reason).await {
                warn!("Error disconnecting {}: {}", device_id, e);
            }
        }
    }
}
