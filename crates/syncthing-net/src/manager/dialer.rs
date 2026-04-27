use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::oneshot;
use tracing::{debug, warn};

use syncthing_core::DeviceId;

use super::ConnectionManager;

impl ConnectionManager {
    /// 连接到设备（含 relay fallback）
    pub(crate) async fn connect_to_with_relay(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
    ) -> syncthing_core::Result<()> {
        // 检查是否已连接
        if self.is_connected(&device_id) {
            debug!("Device {} is already connected", device_id);
            return Ok(());
        }

        // 检查是否已在连接中
        {
            let pending = self.pending_connections.read().await;
            if pending.contains_key(&device_id) {
                debug!("Connection to {} is already pending", device_id);
                return Ok(());
            }
        }

        // 存储地址
        self.device_addresses.insert(device_id, addresses.clone());
        if !relay_urls.is_empty() {
            self.device_relay_urls.insert(device_id, relay_urls.clone());
        }

        // 继承已有重试次数（如果存在）
        let retry_count = {
            let pending = self.pending_connections.read().await;
            pending.get(&device_id).map(|p| p.retry_count).unwrap_or(0)
        };

        // 添加到待连接列表
        let (cancel_tx, cancel_rx) = oneshot::channel();
        {
            let mut pending = self.pending_connections.write().await;
            pending.insert(device_id, super::PendingConnection {
                device_id,
                addresses: addresses.clone(),
                relay_urls: relay_urls.clone(),
                retry_count,
                last_attempt: Some(std::time::Instant::now()),
                _cancel_tx: Some(cancel_tx),
            });
        }

        // 启动连接任务
        self.spawn_connect_task(device_id, addresses, relay_urls, cancel_rx);

        Ok(())
    }
    
    /// 启动连接任务
    fn spawn_connect_task(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
        mut cancel_rx: oneshot::Receiver<()>,
    ) {
        let parallel_dialer = Arc::clone(&self.parallel_dialer);
        let tls_config = Arc::clone(&self.tls_config);
        let local_device_id = self.local_device_id;
        let self_weak = self.self_weak.read().clone().unwrap();

        tokio::spawn(async move {
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
