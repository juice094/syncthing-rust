use std::net::SocketAddr;
use std::sync::Arc;

use syncthing_core::DeviceId;

use crate::connection::BepConnection;

use super::{ConnectionManager, ManagerStats};

/// 连接管理器句柄（用于跨线程共享）
#[derive(Clone)]
pub struct ConnectionManagerHandle {
    pub(crate) inner: Arc<ConnectionManager>,
}

impl ConnectionManagerHandle {
    /// 注册新连接（由传输层调用）
    pub async fn register_connection(
        &self,
        device_id: DeviceId,
        conn: Arc<BepConnection>,
    ) -> syncthing_core::Result<()> {
        self.inner.register_connection(device_id, conn).await
    }

    /// 注册传入连接
    pub async fn register_incoming(&self, conn: Arc<BepConnection>) -> syncthing_core::Result<()> {
        self.inner.register_incoming(conn).await
    }

    /// 获取到指定设备的连接
    pub fn get_connection(&self, device_id: &DeviceId) -> Option<Arc<BepConnection>> {
        self.inner.get_connection(device_id)
    }

    /// 获取所有已连接的设备
    pub fn connected_devices(&self) -> Vec<DeviceId> {
        self.inner.connected_devices()
    }

    /// 断开与设备的连接
    pub async fn disconnect(
        &self,
        device_id: &DeviceId,
        reason: &str,
    ) -> syncthing_core::Result<()> {
        self.inner.disconnect(device_id, reason).await
    }

    /// 断开指定连接
    pub async fn disconnect_connection(
        &self,
        conn_id: &uuid::Uuid,
        reason: &str,
    ) -> syncthing_core::Result<()> {
        self.inner.disconnect_connection(conn_id, reason).await
    }

    /// 按连接ID获取连接
    pub fn get_connection_by_id(&self, conn_id: &uuid::Uuid) -> Option<Arc<BepConnection>> {
        self.inner.get_connection_by_id(conn_id)
    }

    /// 获取实际绑定的监听地址
    pub fn local_addr(&self) -> Option<SocketAddr> {
        *self.inner.listen_addr.read()
    }

    /// 连接到设备
    pub async fn connect_to(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
    ) -> syncthing_core::Result<()> {
        self.inner
            .connect_to_with_relay(device_id, addresses, vec![])
            .await
    }

    /// 连接到设备（含 relay fallback）
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

    /// 按设备 ID 强制重连（自动从内部地址池获取地址）。
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

    /// 强制重连到设备：断开现有连接并重新拨号。
    ///
    /// 与 `connect_to` 不同，此方法在设备已连接时也会执行完整的
    /// 断开 → 清 pending → 拨号流程，确保 `on_connected` 被重新触发。
    pub async fn reconnect_device(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
    ) -> syncthing_core::Result<()> {
        // 1. 断开现有连接（触发 on_disconnected 清理旧 session）
        self.disconnect(&device_id, "reconnect").await.ok();
        // 2. 清除 pending 状态，防止 connect_to_with_relay 因"already pending"直接返回
        {
            let mut pending = self.inner.pending_connections.write().await;
            pending.remove(&device_id);
        }
        // 3. 重新拨号
        self.connect_to_with_relay(device_id, addresses, relay_urls)
            .await
    }

    /// 更新设备的地址池（由 discovery 层调用）
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

    /// 停止连接管理器
    pub async fn stop(&self) -> syncthing_core::Result<()> {
        self.inner.stop().await
    }

    /// 获取统计信息
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
}
