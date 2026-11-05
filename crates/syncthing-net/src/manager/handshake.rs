//! Simultaneous-dial race resolution.
//!
//! See "docs/design/ADR-20261105-connection-race.md" for the race rules, the quiet-period
//! yield rationale, and the replace-stale-connection policy.

use std::sync::Weak;
use std::time::{Duration, Instant};

use tracing::{debug, info};

use syncthing_core::{ConnectionType, DeviceId, SyncthingError};

use super::ConnectionManager;

/// State of a handshake in progress.
#[derive(Debug)]
pub struct HandshakeState {
    /// The connection type this end plays in this handshake.
    pub conn_type: ConnectionType,
    /// When the handshake started.
    pub started_at: Instant,
}

/// Handshake registration guard; cleans up `pending_handshakes` on drop.
pub struct HandshakeGuard {
    manager: Weak<ConnectionManager>,
    device_id: DeviceId,
}

impl HandshakeGuard {
    /// Creates a guard that ends the handshake on drop.
    pub(crate) fn new(manager: Weak<ConnectionManager>, device_id: DeviceId) -> Self {
        Self { manager, device_id }
    }
}

impl Drop for HandshakeGuard {
    fn drop(&mut self) {
        if let Some(manager) = self.manager.upgrade() {
            manager.end_handshake(self.device_id);
        }
    }
}

impl ConnectionManager {
    /// Registers a pending handshake before the TLS / BEP handshake.
    ///
    /// Returns `Err` when this connection should lose the race according to
    /// the device ID rules; the caller must then close the socket immediately
    /// and must not proceed to the BEP Hello.
    ///
    /// Race rules (consistent with Syncthing Go):
    /// - The smaller device ID keeps the **incoming** connection and drops outgoing.
    /// - The larger device ID keeps the **outgoing** connection and drops incoming.
    ///
    /// See "docs/design/ADR-20261105-connection-race.md" for the full decision table.
    pub(crate) fn begin_handshake(
        &self,
        device_id: DeviceId,
        conn_type: ConnectionType,
    ) -> syncthing_core::Result<HandshakeGuard> {
        // Replace any existing connection: a new handshake usually means the peer
        // restarted and the old entry is a zombie. Refusing would deadlock reconnection.
        if self.is_connected(&device_id) {
            info!(
                "device {} already connected, replacing stale connection with new handshake",
                device_id
            );
            if let Some((_, nested)) = self.connections.remove(&device_id) {
                for entry in nested {
                    let (conn_id, e) = entry;
                    self.conn_id_index.remove(&conn_id);
                    tokio::spawn(async move {
                        e.conn.close().await.ok();
                    });
                }
            }
            if let Some(callback) = self.on_disconnected.read().as_ref() {
                callback(device_id, "replaced by new connection".to_string());
            }
        }

        let local_smaller = self.local_device_id.0 < device_id.0;

        match self.pending_handshakes.entry(device_id) {
            dashmap::mapref::entry::Entry::Occupied(o) => {
                let existing = o.get();
                let should_yield_new = match (existing.conn_type, conn_type) {
                    (ConnectionType::Incoming, ConnectionType::Outgoing) => local_smaller,
                    (ConnectionType::Outgoing, ConnectionType::Incoming) => !local_smaller,
                    _ => true,
                };

                if should_yield_new {
                    return Err(SyncthingError::connection(format!(
                        "handshake race lost for {} (existing {:?})",
                        device_id, existing.conn_type
                    )));
                }

                o.remove();
            }
            dashmap::mapref::entry::Entry::Vacant(_) => {}
        }

        self.pending_handshakes.insert(
            device_id,
            HandshakeState {
                conn_type,
                started_at: Instant::now(),
            },
        );

        let weak = self.self_weak().map_err(|_| {
            SyncthingError::internal("connection manager self_weak not initialized")
        })?;

        Ok(HandshakeGuard::new(weak, device_id))
    }

    /// Removes the finished handshake record.
    pub(crate) fn end_handshake(&self, device_id: DeviceId) {
        self.pending_handshakes.remove(&device_id);
    }
}

/// Quiet-period yield before an outgoing handshake.
///
/// When the local device ID is smaller than the remote one, waits a
/// deterministic pseudo-random 50–250 ms before sending the TLS ClientHello,
/// giving the larger-ID side priority to complete its outgoing handshake and
/// lowering the chance of simultaneous ClientHellos. Rationale and the
/// determinism constraint: see "docs/design/ADR-20261105-connection-race.md".
pub async fn pre_handshake_yield(local_device_id: DeviceId, remote_device_id: DeviceId) {
    if local_device_id.0 < remote_device_id.0 {
        let delay_ms = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            local_device_id.0.hash(&mut hasher);
            remote_device_id.0.hash(&mut hasher);
            let hash = hasher.finish();
            50 + (hash % 201)
        };
        debug!(
            "Local device ID smaller than remote {}; yielding {}ms before ClientHello",
            remote_device_id, delay_ms
        );
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::TlsIdentity;
    use crate::manager::config::ConnectionManagerConfig;
    use crate::tls::SyncthingTlsConfig;
    use std::sync::Arc;

    fn test_manager() -> Arc<ConnectionManager> {
        let tls_config = Arc::new(SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
            let (cert, key) = crate::tls::generate_certificate("handshake-test").unwrap();
            SyncthingTlsConfig::from_pem(&cert, &key).unwrap()
        }));
        let identity = Arc::new(TlsIdentity::new(Arc::clone(&tls_config)));
        let (manager, _handle) =
            ConnectionManager::new(ConnectionManagerConfig::default(), identity, tls_config);
        manager
    }

    #[test]
    fn test_begin_handshake_incoming_then_outgoing_local_smaller() {
        let manager = test_manager();
        let local = manager.local_device_id;
        let remote = remote_larger(local);
        assert!(local.0 < remote.0, "test setup: local should be smaller");

        // local 较小，先注册 incoming
        let _g1 = manager
            .begin_handshake(remote, ConnectionType::Incoming)
            .unwrap();
        // 再注册 outgoing 应该失败（local 较小应保留 incoming）
        let err = manager.begin_handshake(remote, ConnectionType::Outgoing);
        assert!(err.is_err(), "local smaller should keep incoming");
    }

    #[test]
    fn test_begin_handshake_outgoing_then_incoming_local_smaller_allowed() {
        let manager = test_manager();
        let local = manager.local_device_id;
        let remote = remote_larger(local);
        assert!(local.0 < remote.0, "test setup: local should be smaller");

        // local 较小，先注册 outgoing
        let _g1 = manager
            .begin_handshake(remote, ConnectionType::Outgoing)
            .unwrap();
        // 新 incoming 不应被错误拒绝（后续 register_connection 会按规则关闭 outgoing）
        let g2 = manager.begin_handshake(remote, ConnectionType::Incoming);
        assert!(
            g2.is_ok(),
            "incoming should be allowed to challenge outgoing when local smaller"
        );
    }

    #[test]
    fn test_begin_handshake_outgoing_then_incoming_local_larger() {
        let manager = test_manager();
        let local = manager.local_device_id;
        let remote = remote_smaller(local);
        assert!(local.0 > remote.0, "test setup: local should be larger");

        // local 较大，先注册 outgoing
        let _g1 = manager
            .begin_handshake(remote, ConnectionType::Outgoing)
            .unwrap();
        // 再注册 incoming 应该失败（local 较大应保留 outgoing）
        let err = manager.begin_handshake(remote, ConnectionType::Incoming);
        assert!(err.is_err(), "local larger should keep outgoing");
    }

    #[test]
    fn test_guard_clears_pending_handshake() {
        let manager = test_manager();
        let remote = DeviceId::default();

        {
            let _g = manager
                .begin_handshake(remote, ConnectionType::Outgoing)
                .unwrap();
            assert!(manager.pending_handshakes.contains_key(&remote));
        }

        assert!(!manager.pending_handshakes.contains_key(&remote));
    }

    fn remote_larger(local: DeviceId) -> DeviceId {
        let mut bytes = local.0;
        for i in 0..32 {
            if bytes[i] < 255 {
                bytes[i] += 1;
                return DeviceId::from_bytes(&bytes).unwrap();
            }
        }
        panic!("local device ID is max");
    }

    fn remote_smaller(local: DeviceId) -> DeviceId {
        let mut bytes = local.0;
        for i in 0..32 {
            if bytes[i] > 0 {
                bytes[i] -= 1;
                return DeviceId::from_bytes(&bytes).unwrap();
            }
        }
        panic!("local device ID is zero");
    }
}
