use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::oneshot;
use tracing::{debug, warn};

use syncthing_core::{ConnectionType, DeviceId};

use super::ConnectionManager;

impl ConnectionManager {
    /// 连接到设备（含 relay fallback）
    pub(crate) async fn connect_to_with_relay(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
    ) -> syncthing_core::Result<()> {
        // 持久化地址和 relay URL（必须在 is_connected 检查之前，
        // 否则已有直连时 relay URL 不会被存储，断开后 dial 无 relay 候选）
        self.device_addresses.insert(device_id, addresses.clone());
        if !relay_urls.is_empty() {
            self.device_relay_urls.insert(device_id, relay_urls.clone());
        }

        // 在单个 write 临界区内原子完成：
        // 1. 检查是否已连接
        // 2. 检查是否已在连接中
        // 3. 继承重试次数
        // 4. 插入 pending 表
        // 防止并发网络事件（netmon 重连 + 定时重试）导致同一设备并行拨号风暴。
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
                    addresses: addresses.clone(),
                    relay_urls: relay_urls.clone(),
                    retry_count,
                    last_attempt: Some(std::time::Instant::now()),
                    _cancel_tx: Some(cancel_tx),
                },
            );
            cancel_rx
        };

        // 在 TLS ClientHello 前注册握手竞争状态。
        // 若根据 device ID 竞争规则本端应保留 incoming，则直接放弃 outgoing 拨号，
        // 避免无效的双向 TLS ClientHello / BEP Hello。
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

        // 启动连接任务（必须在释放 pending write lock 之后，避免任务执行时死锁）
        self.spawn_connect_task(device_id, addresses, relay_urls, cancel_rx, handshake_guard);

        Ok(())
    }

    /// 启动连接任务
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
            // 保持握手守卫直到拨号任务结束，在 BEP Hello / 注册前避免竞争。
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
