//! 握手阶段竞争解决
//!
//! 将连接竞争解决从 `register_connection` 提前到 TLS 握手完成后、BEP Hello 前，
//! 避免无效的双向 BEP Hello / session 启动。同时提供“静默期”退让，降低两端同时
//! 发送 TLS ClientHello 的概率。

use std::sync::Weak;
use std::time::{Duration, Instant};

use tracing::debug;

use syncthing_core::{ConnectionType, DeviceId, SyncthingError};

use super::ConnectionManager;

/// 进行中的握手状态
#[derive(Debug)]
pub struct HandshakeState {
    /// 本端在这场握手中扮演的连接类型
    pub conn_type: ConnectionType,
    /// 握手开始时间
    pub started_at: Instant,
}

/// 握手注册守卫，离开作用域时自动清理 `pending_handshakes`。
pub struct HandshakeGuard {
    manager: Weak<ConnectionManager>,
    device_id: DeviceId,
}

impl HandshakeGuard {
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
    /// 在 TLS / BEP 握手前注册一个待完成的握手。
    ///
    /// 如果根据 device ID 竞争规则本连接应当失败，则返回 `Err`，调用方应立即关闭
    /// 已建立的 socket，不要继续 BEP Hello。
    ///
    /// 竞争规则（与 Syncthing Go 一致）：
    /// - device ID 较小的一方保留 **incoming** 连接，关闭 outgoing；
    /// - device ID 较大的一方保留 **outgoing** 连接，关闭 incoming。
    pub(crate) fn begin_handshake(
        &self,
        device_id: DeviceId,
        conn_type: ConnectionType,
    ) -> syncthing_core::Result<HandshakeGuard> {
        // 已有存活连接时直接拒绝新的握手尝试。
        if self.is_connected(&device_id) {
            return Err(SyncthingError::connection(format!(
                "device {} already connected",
                device_id
            )));
        }

        let local_smaller = self.local_device_id.0 < device_id.0;

        match self.pending_handshakes.entry(device_id) {
            dashmap::mapref::entry::Entry::Occupied(o) => {
                let existing = o.get();
                let should_yield_new = match (existing.conn_type, conn_type) {
                    // 旧 incoming + 新 outgoing：local 较小者应保留 incoming → 新 outgoing 失败
                    (ConnectionType::Incoming, ConnectionType::Outgoing) => local_smaller,
                    // 旧 outgoing + 新 incoming：local 较大者应保留 outgoing → 新 incoming 失败
                    (ConnectionType::Outgoing, ConnectionType::Incoming) => !local_smaller,
                    // 同类型：先开始的获胜，避免同一方向重复握手
                    _ => true,
                };

                if should_yield_new {
                    return Err(SyncthingError::connection(format!(
                        "handshake race lost for {} (existing {:?})",
                        device_id, existing.conn_type
                    )));
                }

                // 新的握手应该获胜：移除旧的记录，让旧握手在后续注册阶段自然失败。
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

    /// 清理已结束的握手记录。
    pub(crate) fn end_handshake(&self, device_id: DeviceId) {
        self.pending_handshakes.remove(&device_id);
    }
}

///  outgoing 握手前的“静默期”退让。
///
/// 当本端 device ID 小于对端时，在发送 TLS ClientHello 前随机等待一小段时间，
/// 给 device ID 较大的一方优先完成 outgoing 握手的机会，从而降低两端同时发起
/// TLS ClientHello 的概率。
///
/// 随机范围 50~250ms，足够覆盖局域网/同地区 relay 的 RTT，又不会让首次连接
/// 显得明显迟缓。
pub async fn pre_handshake_yield(local_device_id: DeviceId, remote_device_id: DeviceId) {
    if local_device_id.0 < remote_device_id.0 {
        // 使用确定性伪随机延迟（基于双方设备 ID 哈希），避免 `rand::thread_rng()`
        // 跨 await 导致 future 非 Send 的问题。
        let delay_ms = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            local_device_id.0.hash(&mut hasher);
            remote_device_id.0.hash(&mut hasher);
            let hash = hasher.finish();
            50 + (hash % 201) // 50..250 ms（含）
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
