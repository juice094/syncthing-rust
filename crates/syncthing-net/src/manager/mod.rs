//! 连接管理器
//!
//! 管理所有活跃连接，处理连接建立、断开和重连。
//! 参考: syncthing/lib/connections/service.go, registry.go

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use std::time::Instant;

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

/// 连接管理器
pub struct ConnectionManager {
    /// 配置
    pub(crate) config: ConnectionManagerConfig,
    /// 本地设备身份（抽象层，解耦具体密码学方案）
    pub(crate) identity: Arc<dyn Identity>,
    /// 本地设备ID（从 identity 缓存，避免虚函数调用）
    pub(crate) local_device_id: DeviceId,
    /// 活跃连接池 (device_id -> conn_id -> connection)
    pub(crate) connections: DashMap<DeviceId, DashMap<uuid::Uuid, ConnectionEntry>>,
    /// 按连接ID索引 (conn_id -> device_id)
    pub(crate) conn_id_index: DashMap<uuid::Uuid, DeviceId>,
    /// 待连接设备
    pub(crate) pending_connections: TokioRwLock<HashMap<DeviceId, PendingConnection>>,
    /// 设备地址映射
    pub(crate) device_addresses: DashMap<DeviceId, Vec<SocketAddr>>,
    /// 设备 Relay URL 映射
    pub(crate) device_relay_urls: DashMap<DeviceId, Vec<String>>,
    /// 事件发送器
    pub(crate) event_tx: mpsc::UnboundedSender<ConnectionEvent>,
    /// 事件接收器
    pub(crate) event_rx: RwLock<Option<mpsc::UnboundedReceiver<ConnectionEvent>>>,
    /// 运行状态
    pub(crate) running: RwLock<bool>,
    /// 维护任务句柄
    pub(crate) maintenance_handle: RwLock<Option<JoinHandle<()>>>,
    /// 网络监控任务句柄
    pub(crate) netmon_handle: RwLock<Option<JoinHandle<()>>>,
    /// 连接回调
    pub(crate) on_connected: RwLock<Option<ConnectedCallback>>,
    /// 断开回调
    pub(crate) on_disconnected: RwLock<Option<DisconnectedCallback>>,
    /// 自引用弱指针
    pub(crate) self_weak: RwLock<Option<Weak<ConnectionManager>>>,
    /// 并行拨号器
    pub(crate) parallel_dialer: Arc<ParallelDialer>,
    /// TLS 配置（供传输层握手使用，Phase 2 后将由 Transport trait 自行管理）
    pub(crate) tls_config: Arc<SyncthingTlsConfig>,
    /// 传输层注册表（Phase 2：支持多传输可插拔）
    pub(crate) transport_registry: RwLock<Option<Arc<TransportRegistry>>>,
    /// 实际绑定的监听地址
    pub(crate) listen_addr: RwLock<Option<SocketAddr>>,
}

impl ConnectionManager {
    /// 创建新的连接管理器
    pub fn new(
        config: ConnectionManagerConfig,
        identity: Arc<dyn Identity>,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> (Arc<Self>, ConnectionManagerHandle) {
        let local_device_id = identity.device_id();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let parallel_dialer = Arc::new(ParallelDialer::with_tcp_connector(
            local_device_id,
            "syncthing-rust".to_string(),
        ));

        let manager = Arc::new_cyclic(|weak| {
            Self {
                config,
                identity,
                local_device_id,
                connections: DashMap::new(),
                conn_id_index: DashMap::new(),
                pending_connections: TokioRwLock::new(HashMap::new()),
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
            }
        });

        let handle = ConnectionManagerHandle {
            inner: Arc::clone(&manager),
        };

        (manager, handle)
    }

    /// 设置连接回调
    pub fn on_connected<F>(&self, callback: F)
    where
        F: Fn(DeviceId) + Send + Sync + 'static,
    {
        *self.on_connected.write() = Some(Arc::new(callback));
    }

    /// 设置断开回调
    pub fn on_disconnected<F>(&self, callback: F)
    where
        F: Fn(DeviceId, String) + Send + Sync + 'static,
    {
        *self.on_disconnected.write() = Some(Arc::new(callback));
    }

    /// 设置传输层注册表（必须在 start() 之前调用）
    pub fn set_transport_registry(&self, registry: Arc<TransportRegistry>) {
        // 若注册表包含默认传输，同步更新 ParallelDialer 的连接器
        if let Some(transport) = registry.default_transport() {
            let connector = Arc::new(crate::transport::bep_adapter::TransportBepConnector::new(transport));
            self.parallel_dialer.set_connector(connector);
        }
        *self.transport_registry.write() = Some(registry);
        info!("Transport registry set with schemes: {:?}", self.transport_registry.read().as_ref().unwrap().schemes());
    }

    /// 启动连接管理器
    pub async fn start(&self) -> syncthing_core::Result<SocketAddr> {
        if *self.running.read() {
            return Err(SyncthingError::config("connection manager already running"));
        }

        info!("Starting connection manager...");

        let self_weak = self.self_weak.read().clone()
            .ok_or_else(|| SyncthingError::config("connection manager not properly initialized"))?;
        let handle = ConnectionManagerHandle { inner: self_weak.upgrade()
            .ok_or_else(|| SyncthingError::config("connection manager dropped"))? };

        // Phase 2：优先使用 TransportRegistry，否则回退到旧式 TcpTransport
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
            ).await {
                Ok(addr) => addr,
                Err(e) if self.config.listen_addr.port() != 0 => {
                    warn!("Transport listener failed to bind to {}, trying random port: {}", self.config.listen_addr, e);
                    let fallback_addr = "0.0.0.0:0".to_string();
                    crate::transport::bep_adapter::BepTransportListener::start(
                        transport,
                        &fallback_addr,
                        handle,
                        self.local_device_id,
                        "syncthing-rust".to_string(),
                        Arc::clone(&self.tls_config),
                    ).await?
                }
                Err(e) => return Err(e),
            }
        } else {
            // 回退：旧式 TcpTransport（Phase 1 行为）
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
                    warn!("Failed to bind TCP listener to {}, trying random port: {}", self.config.listen_addr, e);
                    let fallback_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
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

        // 启动事件处理任务
        self.spawn_event_handler();

        // 启动维护任务
        self.spawn_maintenance_task();

        *self.listen_addr.write() = Some(listen_addr);

        info!("Connection manager started on {}", listen_addr);

        Ok(listen_addr)
    }

    /// 停止连接管理器
    pub async fn stop(&self) -> syncthing_core::Result<()> {
        info!("Stopping connection manager...");

        *self.running.write() = false;

        // 停止网络监控任务
        if let Some(handle) = self.netmon_handle.write().take() {
            handle.abort();
        }

        // 停止维护任务
        if let Some(handle) = self.maintenance_handle.write().take() {
            handle.abort();
        }

        // 断开所有连接
        self.disconnect_all("manager shutting down").await;

        info!("Connection manager stopped");
        Ok(())
    }

    /// 获取连接（返回首个存活连接）
    pub(crate) fn get_connection(&self, device_id: &DeviceId) -> Option<Arc<BepConnection>> {
        self.connections
            .get(device_id)
            .and_then(|nested| {
                nested.iter()
                    .find(|e| e.value().conn.is_alive())
                    .map(|e| Arc::clone(&e.value().conn))
            })
    }

    /// 按连接ID获取连接
    pub(crate) fn get_connection_by_id(&self, conn_id: &uuid::Uuid) -> Option<Arc<BepConnection>> {
        self.conn_id_index.get(conn_id).and_then(|device_id| {
            self.connections.get(&*device_id).and_then(|nested| {
                nested.get(conn_id).map(|e| Arc::clone(&e.conn))
            })
        })
    }

    /// 检查设备是否已连接
    pub fn is_connected(&self, device_id: &DeviceId) -> bool {
        self.get_connection(device_id).is_some()
    }

    /// 获取所有已连接的设备
    pub fn connected_devices(&self) -> Vec<DeviceId> {
        self.connections
            .iter()
            .filter(|entry| entry.value().iter().any(|e| e.value().conn.is_alive()))
            .map(|entry| *entry.key())
            .collect()
    }

    /// 启动网络监控任务
    pub fn start_netmon(&self, mut rx: mpsc::Receiver<NetChangeEvent>) {
        let weak = self.self_weak.read().clone().unwrap();
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

    /// 处理网络变化事件
    async fn handle_net_change(&self, _event: NetChangeEvent) {
        info!("Network change detected, cleaning up stale connections and rebind dialing");

        self.cleanup_stale_connections().await;

        let devices: Vec<DeviceId> = self.device_addresses
            .iter()
            .map(|entry| *entry.key())
            .collect();

        for device_id in devices {
            if !self.is_connected(&device_id) {
                let addresses = self.device_addresses
                    .get(&device_id)
                    .map(|e| e.clone())
                    .unwrap_or_default();
                let relay_urls = self.device_relay_urls
                    .get(&device_id)
                    .map(|e| e.clone())
                    .unwrap_or_default();
                if !addresses.is_empty() || !relay_urls.is_empty() {
                    if let Err(e) = self.connect_to_with_relay(device_id, addresses, relay_urls).await {
                        warn!("Rebind redial to {} failed: {}", device_id, e);
                    }
                }
            }
        }
    }

    /// 检查是否应该重连
    pub(crate) fn should_reconnect(&self, _device_id: &DeviceId, reason: &str) -> bool {
        // 检查断开原因是否值得重试
        !reason.contains("manual disconnect")
            && !reason.contains("invalid device ID")
            && !reason.contains("unauthorized")
    }

    /// 安排重连
    pub(crate) async fn schedule_reconnect(&self, device_id: DeviceId) {
        // 获取地址
        let addresses = self.device_addresses
            .get(&device_id)
            .map(|e| e.clone())
            .unwrap_or_default();
        let relay_urls = self.device_relay_urls
            .get(&device_id)
            .map(|e| e.clone())
            .unwrap_or_default();

        if addresses.is_empty() && relay_urls.is_empty() {
            warn!("No addresses available for device {}, skipping reconnect", device_id);
            return;
        }

        // 增加/设置重试次数
        let retry_count = {
            let mut pending = self.pending_connections.write().await;
            if let Some(p) = pending.get_mut(&device_id) {
                p.retry_count += 1;
                p.retry_count
            } else {
                pending.insert(device_id, PendingConnection {
                    device_id,
                    addresses: addresses.clone(),
                    relay_urls: relay_urls.clone(),
                    retry_count: 1,
                    last_attempt: Some(Instant::now()),
                    _cancel_tx: None,
                });
                1
            }
        };

        // 计算退避时间
        let backoff = self.config.retry_config.backoff_duration(retry_count);

        info!("Scheduling reconnect to {} in {:?} (retry_count={})", device_id, backoff, retry_count);

        // 延迟后重连
        let weak = self.self_weak.read().clone().unwrap();
        tokio::spawn(async move {
            sleep(backoff).await;

            if let Some(manager) = weak.upgrade() {
                // 清除 pending 状态，否则 connect_to 会因"already pending"而直接返回
                manager.pending_connections.write().await.remove(&device_id);
                if let Err(e) = manager.connect_to_with_relay(device_id, addresses, relay_urls).await {
                    warn!("Scheduled reconnect to {} failed: {}", device_id, e);
                }
            }
        });
    }

    /// 清理过期连接
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
            if let Err(e) = self.disconnect_connection(&conn_id, "stale connection").await {
                warn!("Error disconnecting stale connection {}: {}", conn_id, e);
            }
        }
    }

    /// 获取统计信息
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
            pending_connections: 0, // 简化处理
            total_bytes_sent,
            total_bytes_received,
        }
    }

    #[allow(dead_code)]
    /// 从引用创建（用于内部转换）
    fn from_arc(manager: &ConnectionManager) -> Self {
        let (_event_tx, _event_rx) = mpsc::unbounded_channel::<ConnectionEvent>();

        Self {
            config: manager.config.clone(),
            identity: Arc::clone(&manager.identity),
            local_device_id: manager.local_device_id,
            connections: manager.connections.clone(),
            conn_id_index: manager.conn_id_index.clone(),
            pending_connections: TokioRwLock::new(HashMap::new()),
            device_addresses: manager.device_addresses.clone(),
            device_relay_urls: manager.device_relay_urls.clone(),
            event_tx: manager.event_tx.clone(),
            event_rx: RwLock::new(None),
            running: RwLock::new(*manager.running.read()),
            maintenance_handle: RwLock::new(None),
            netmon_handle: RwLock::new(None),
            on_connected: RwLock::new(manager.on_connected.read().clone()),
            on_disconnected: RwLock::new(manager.on_disconnected.read().clone()),
            self_weak: RwLock::new(None),
            parallel_dialer: Arc::clone(&manager.parallel_dialer),
            tls_config: Arc::clone(&manager.tls_config),
            transport_registry: RwLock::new(manager.transport_registry.read().clone()),
            listen_addr: RwLock::new(*manager.listen_addr.read()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use crate::tcp_transport::DEFAULT_TCP_PORT;

    use super::*;

    #[test]
    fn test_connection_manager_config_default() {
        let config = ConnectionManagerConfig::default();
        assert_eq!(config.listen_addr.port(), DEFAULT_TCP_PORT);
        assert_eq!(config.max_connections, 1000);
    }

    #[tokio::test]
    async fn test_rebind_triggers_redial() {
        let tls_config = Arc::new(
            SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
                let (cert, key) = crate::tls::generate_certificate("syncthing-rust-test")
                    .expect("failed to generate certificate");
                SyncthingTlsConfig::from_pem(&cert, &key)
                    .expect("failed to load generated certificate")
            })
        );
        let identity = Arc::new(crate::identity::TlsIdentity::new(Arc::clone(&tls_config)));
        let (manager, _handle) = ConnectionManager::new(
            ConnectionManagerConfig::default(),
            identity,
            tls_config,
        );

        // Register a device address but don't connect
        let device_id = DeviceId::default();
        let addr: SocketAddr = "127.0.0.1:22001".parse().unwrap();
        manager.device_addresses.insert(device_id, vec![addr]);

        // Manually trigger network change handling
        manager.handle_net_change(NetChangeEvent::InterfacesChanged).await;

        // Verify pending connection was created
        let pending = manager.pending_connections.read().await;
        assert!(pending.contains_key(&device_id));
    }

    #[tokio::test]
    async fn test_transport_registry_start_listen() {
        // Phase 2 验证：ConnectionManager 通过 TransportRegistry 启动监听
        let tls_config = Arc::new(
            SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
                let (cert, key) = crate::tls::generate_certificate("transport-registry-test")
                    .expect("failed to generate certificate");
                SyncthingTlsConfig::from_pem(&cert, &key)
                    .expect("failed to load generated certificate")
            })
        );
        let identity = Arc::new(crate::identity::TlsIdentity::new(Arc::clone(&tls_config)));
        let config = ConnectionManagerConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        };
        let (manager, _handle) = ConnectionManager::new(config, identity, tls_config);

        // 注册 TransportRegistry
        let mut registry = crate::transport::TransportRegistry::new();
        registry.register(Arc::new(crate::transport::RawTcpTransport::new()));
        manager.set_transport_registry(Arc::new(registry));

        // 启动监听
        let addr = manager.start().await.expect("failed to start with TransportRegistry");
        assert!(addr.port() > 0, "should bind to a random port");

        // 清理
        manager.stop().await.expect("failed to stop");
    }
}
