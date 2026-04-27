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
    pub async fn register_connection(&self, device_id: DeviceId, conn: Arc<BepConnection>) -> syncthing_core::Result<()> {
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
    pub async fn disconnect(&self, device_id: &DeviceId, reason: &str) -> syncthing_core::Result<()> {
        self.inner.disconnect(device_id, reason).await
    }

    /// 断开指定连接
    pub async fn disconnect_connection(&self, conn_id: &uuid::Uuid, reason: &str) -> syncthing_core::Result<()> {
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
    pub async fn connect_to(&self, device_id: DeviceId, addresses: Vec<SocketAddr>) -> syncthing_core::Result<()> {
        self.inner.connect_to_with_relay(device_id, addresses, vec![]).await
    }

    /// 连接到设备（含 relay fallback）
    pub async fn connect_to_with_relay(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
        relay_urls: Vec<String>,
    ) -> syncthing_core::Result<()> {
        self.inner.connect_to_with_relay(device_id, addresses, relay_urls).await
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

    async fn disconnect(&self, device_id: &syncthing_core::DeviceId, reason: &str) -> syncthing_core::Result<()> {
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

    fn get_connection_info(&self, device_id: &syncthing_core::DeviceId) -> Option<syncthing_core::traits::ConnectionInfo> {
        self.get_connection(device_id).map(|conn| syncthing_core::traits::ConnectionInfo {
            remote_addr: conn.remote_addr().to_string(),
            is_alive: conn.is_alive(),
        })
    }
}
