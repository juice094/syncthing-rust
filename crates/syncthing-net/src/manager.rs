//! 连接管理器
//!
//! 管理所有活跃连接，处理连接建立、断开和重连
//! 参考: syncthing/lib/connections/service.go, registry.go

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::sync::{mpsc, oneshot, RwLock as TokioRwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep};
use tracing::{debug, info, warn};

use syncthing_core::{
    ConnectionType, DeviceId, RetryConfig, SyncthingError
};

use crate::connection::{BepConnection, ConnectionEvent};
use crate::dialer::ParallelDialer;
use crate::netmon::NetChangeEvent;
use crate::tcp_transport::{TcpTransport, DEFAULT_TCP_PORT};
use crate::tls::SyncthingTlsConfig;

/// 连接管理器配置
#[derive(Debug, Clone)]
pub struct ConnectionManagerConfig {
    /// 监听地址
    pub listen_addr: SocketAddr,
    /// 重试配置
    pub retry_config: RetryConfig,
    /// 心跳间隔
    pub heartbeat_interval: Duration,
    /// 连接超时
    pub connection_timeout: Duration,
    /// 最大并发连接数
    pub max_connections: usize,
}

impl Default for ConnectionManagerConfig {
    fn default() -> Self {
        Self {
            listen_addr: ([0, 0, 0, 0], DEFAULT_TCP_PORT).into(),
            retry_config: RetryConfig::default(),
            heartbeat_interval: Duration::from_secs(90),
            connection_timeout: Duration::from_secs(120),
            max_connections: 1000,
        }
    }
}

/// 连接条目
#[derive(Clone)]
struct ConnectionEntry {
    /// 连接对象
    conn: Arc<BepConnection>,
    /// 连接建立时间
    connected_at: Instant,
    /// 重试次数
    retry_count: u32,
}

impl ConnectionEntry {
    fn new(conn: Arc<BepConnection>) -> Self {
        Self {
            conn,
            connected_at: Instant::now(),
            retry_count: 0,
        }
    }

    fn is_stale(&self, timeout: Duration) -> bool {
        self.conn
            .last_activity_age()
            .map_or(true, |age| age > timeout)
    }
}

/// 待连接设备
struct PendingConnection {
    device_id: DeviceId,
    addresses: Vec<SocketAddr>,
    retry_count: u32,
    last_attempt: Option<Instant>,
    // 重试任务的取消句柄
    _cancel_tx: Option<oneshot::Sender<()>>,
}

/// 连接管理器
pub struct ConnectionManager {
    /// 配置
    config: ConnectionManagerConfig,
    /// 本地设备ID
    local_device_id: DeviceId,
    /// 活跃连接池 (device_id -> connection)
    connections: DashMap<DeviceId, ConnectionEntry>,
    /// 按连接ID索引 (conn_id -> device_id)
    conn_id_index: DashMap<uuid::Uuid, DeviceId>,
    /// 待连接设备
    pending_connections: TokioRwLock<HashMap<DeviceId, PendingConnection>>,
    /// 设备地址映射
    device_addresses: DashMap<DeviceId, Vec<SocketAddr>>,
    /// 事件发送器
    event_tx: mpsc::UnboundedSender<ConnectionEvent>,
    /// 事件接收器
    event_rx: RwLock<Option<mpsc::UnboundedReceiver<ConnectionEvent>>>,
    /// 运行状态
    running: RwLock<bool>,
    /// 维护任务句柄
    maintenance_handle: RwLock<Option<JoinHandle<()>>>,
    /// 网络监控任务句柄
    netmon_handle: RwLock<Option<JoinHandle<()>>>,
    /// 连接回调
    on_connected: RwLock<Option<Arc<dyn Fn(DeviceId) + Send + Sync>>>,
    /// 断开回调
    on_disconnected: RwLock<Option<Arc<dyn Fn(DeviceId, String) + Send + Sync>>>,
    /// 自引用弱指针
    self_weak: RwLock<Option<Weak<ConnectionManager>>>,
    /// 并行拨号器
    parallel_dialer: Arc<ParallelDialer>,
    /// TLS 配置
    tls_config: Arc<SyncthingTlsConfig>,
}

/// 连接管理器句柄（用于跨线程共享）
#[derive(Clone)]
pub struct ConnectionManagerHandle {
    inner: Arc<ConnectionManager>,
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
    
    /// 连接到设备
    pub async fn connect_to(&self, device_id: DeviceId, addresses: Vec<SocketAddr>) -> syncthing_core::Result<()> {
        self.inner.connect_to(device_id, addresses).await
    }

    /// 获取统计信息
    pub fn stats(&self) -> ManagerStats {
        self.inner.stats()
    }
}

impl ConnectionManager {
    /// 创建新的连接管理器
    pub fn new(
        config: ConnectionManagerConfig,
        local_device_id: DeviceId,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> (Arc<Self>, ConnectionManagerHandle) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        let parallel_dialer = Arc::new(ParallelDialer::with_tcp_connector(
            local_device_id,
            "syncthing-rust".to_string(),
        ));
        
        let manager = Arc::new_cyclic(|weak| {
            Self {
                config,
                local_device_id,
                connections: DashMap::new(),
                conn_id_index: DashMap::new(),
                pending_connections: TokioRwLock::new(HashMap::new()),
                device_addresses: DashMap::new(),
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
        
        let mut tcp_transport = TcpTransport::new(
            self.config.listen_addr,
            handle.clone(),
            self.local_device_id,
            "syncthing-rust".to_string(),
            Arc::clone(&self.tls_config),
        );
        
        // 启动TCP监听，失败时回退到随机端口
        let listen_addr = match tcp_transport.start().await {
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
        };
        
        *self.running.write() = true;
        
        // 启动事件处理任务
        self.spawn_event_handler();
        
        // 启动维护任务
        self.spawn_maintenance_task();
        
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
    
    /// 注册新连接
    async fn register_connection(&self, device_id: DeviceId, conn: Arc<BepConnection>) -> syncthing_core::Result<()> {
        debug!("Registering connection for device {}", device_id);
        
        // 检查是否已存在连接
        if let Some(existing) = self.connections.get(&device_id) {
            // 比较连接优先级，决定是否替换
            if self.should_replace_connection(&existing.conn, &conn) {
                info!("Replacing existing connection for device {}", device_id);
                existing.conn.close().await.ok();
            } else {
                // 关闭新连接
                conn.close().await.ok();
                return Err(SyncthingError::connection("existing connection has higher priority"));
            }
        }
        
        // 添加新连接
        let entry = ConnectionEntry::new(Arc::clone(&conn));
        self.connections.insert(device_id, entry);
        self.conn_id_index.insert(conn.id(), device_id);
        
        // 设置连接的设备ID
        conn.set_device_id(device_id);
        
        // 从待连接列表中移除
        self.pending_connections.write().await.remove(&device_id);
        
        info!("Connection registered for device {} (conn_id: {})", device_id, conn.id());
        
        // 触发回调
        if let Some(callback) = self.on_connected.read().as_ref() {
            callback(device_id);
        }
        
        // 发送事件
        let _ = self.event_tx.send(ConnectionEvent::Connected {
            device_id,

        });
        
        Ok(())
    }
    
    /// 注册传入连接（设备ID已知时直接注册）
    async fn register_incoming(&self, conn: Arc<BepConnection>) -> syncthing_core::Result<()> {
        debug!("Registering incoming connection {}", conn.id());
        
        if let Some(device_id) = conn.device_id() {
            self.register_connection(device_id, conn).await
        } else {
            warn!("Incoming connection {} has no device_id, skipping registration", conn.id());
            Err(SyncthingError::connection("incoming connection missing device ID"))
        }
    }
    
    /// 获取连接
    fn get_connection(&self, device_id: &DeviceId) -> Option<Arc<BepConnection>> {
        self.connections
            .get(device_id)
            .filter(|entry| entry.conn.is_alive())
            .map(|entry| Arc::clone(&entry.conn))
    }
    
    /// 检查设备是否已连接
    pub fn is_connected(&self, device_id: &DeviceId) -> bool {
        self.get_connection(device_id).is_some()
    }
    
    /// 获取所有已连接的设备
    pub fn connected_devices(&self) -> Vec<DeviceId> {
        self.connections
            .iter()
            .filter(|entry| entry.conn.is_alive())
            .map(|entry| *entry.key())
            .collect()
    }
    
    /// 断开与设备的连接
    async fn disconnect(&self, device_id: &DeviceId, reason: &str) -> syncthing_core::Result<()> {
        info!("Disconnecting device {}: {}", device_id, reason);

        if let Some((_, entry)) = self.connections.remove(device_id) {
            entry.conn.close().await?;
            self.conn_id_index.remove(&entry.conn.id());

            // 触发回调
            if let Some(callback) = self.on_disconnected.read().as_ref() {
                callback(*device_id, reason.to_string());
            }
        }

        // 触发重连（如果适用）
        if self.should_reconnect(device_id, reason) {
            self.schedule_reconnect(*device_id).await;
        }

        Ok(())
    }
    
    /// 断开所有连接
    async fn disconnect_all(&self, reason: &str) {
        let devices: Vec<DeviceId> = self.connections.iter().map(|e| *e.key()).collect();
        
        for device_id in devices {
            if let Err(e) = self.disconnect(&device_id, reason).await {
                warn!("Error disconnecting {}: {}", device_id, e);
            }
        }
    }
    
    /// 连接到设备
    async fn connect_to(&self, device_id: DeviceId, addresses: Vec<SocketAddr>) -> syncthing_core::Result<()> {
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

        // 继承已有重试次数（如果存在）
        let retry_count = {
            let pending = self.pending_connections.read().await;
            pending.get(&device_id).map(|p| p.retry_count).unwrap_or(0)
        };

        // 添加到待连接列表
        let (cancel_tx, cancel_rx) = oneshot::channel();
        {
            let mut pending = self.pending_connections.write().await;
            pending.insert(device_id, PendingConnection {
                device_id,
                addresses: addresses.clone(),
                retry_count,
                last_attempt: Some(Instant::now()),
                _cancel_tx: Some(cancel_tx),
            });
        }

        // 启动连接任务
        self.spawn_connect_task(device_id, addresses, cancel_rx);

        Ok(())
    }
    
    /// 启动连接任务
    fn spawn_connect_task(
        &self,
        device_id: DeviceId,
        addresses: Vec<SocketAddr>,
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
    
    /// 启动事件处理任务
    fn spawn_event_handler(&self) {
        let mut event_rx = self.event_rx.write().take()
            .expect("event receiver already taken");
        
        let weak = self.self_weak.read().clone().unwrap();
        
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                if let Some(manager) = weak.upgrade() {
                    manager.handle_event(event).await;
                } else {
                    break;
                }
            }
        });
    }
    
    /// 处理连接事件
    async fn handle_event(&self, event: ConnectionEvent) {
        match event {
            ConnectionEvent::Connected { device_id } => {
                debug!("Device {} connected", device_id);
            }
            ConnectionEvent::Disconnected { reason } => {
                // 需要从连接中找到设备ID
                let _device_id: Option<DeviceId> = None; // 简化处理
                info!("Device disconnected: {:?} - {}", _device_id, reason);
                
                // 从活跃连接中移除
                // self.connections.remove(&device_id); 简化处理
                
                // 触发重连（如果适用）
                if let Some(ref d) = _device_id {
                    if self.should_reconnect(d, &reason) {
                        self.schedule_reconnect(*d).await;
                    }
                    
                    // 触发回调
                    if let Some(callback) = self.on_disconnected.read().as_ref() {
                        callback(*d, reason.clone());
                    }
                }
            }
            ConnectionEvent::Error { error } => {
                warn!("Connection error: {}", error);
            }
            _ => {}
        }
    }
    
    /// 启动维护任务
    fn spawn_maintenance_task(&self) {
        let interval_duration = self.config.heartbeat_interval;
        
        let weak = self.self_weak.read().clone().unwrap();
        
        let handle = tokio::spawn(async move {
            let mut ticker = interval(interval_duration);
            
            loop {
                ticker.tick().await;
                if let Some(manager) = weak.upgrade() {
                    manager.cleanup_stale_connections().await;
                } else {
                    break;
                }
            }
        });
        
        *self.maintenance_handle.write() = Some(handle);
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
                if !addresses.is_empty() {
                    if let Err(e) = self.connect_to(device_id, addresses).await {
                        warn!("Rebind redial to {} failed: {}", device_id, e);
                    }
                }
            }
        }
    }
    
    /// 检查是否应该替换现有连接
    fn should_replace_connection(&self, existing: &BepConnection, new: &BepConnection) -> bool {
        // 优先保留传出连接（通常更可靠）
        match (existing.connection_type(), new.connection_type()) {
            (ConnectionType::Outgoing, ConnectionType::Incoming) => false,
            (ConnectionType::Incoming, ConnectionType::Outgoing) => true,
            _ => {
                // 相同类型时，优先保留较新的连接
                // 简化处理：保留新连接
                true
            }
        }
    }
    
    /// 检查是否应该重连
    fn should_reconnect(&self, _device_id: &DeviceId, reason: &str) -> bool {
        // 检查断开原因是否值得重试
        !reason.contains("manual disconnect")
            && !reason.contains("invalid device ID")
            && !reason.contains("unauthorized")
    }
    
    /// 安排重连
    async fn schedule_reconnect(&self, device_id: DeviceId) {
        // 获取地址
        let addresses = self.device_addresses
            .get(&device_id)
            .map(|e| e.clone())
            .unwrap_or_default();

        if addresses.is_empty() {
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
                if let Err(e) = manager.connect_to(device_id, addresses).await {
                    warn!("Scheduled reconnect to {} failed: {}", device_id, e);
                }
            }
        });
    }
    
    /// 清理过期连接
    async fn cleanup_stale_connections(&self) {
        let stale_threshold = self.config.connection_timeout;
        let stale_devices: Vec<DeviceId> = self.connections
            .iter()
            .filter(|entry| !entry.conn.is_alive() || entry.is_stale(stale_threshold))
            .map(|entry| *entry.key())
            .collect();
        
        for device_id in stale_devices {
            warn!("Connection to {} is stale or dead, disconnecting", device_id);
            if let Err(e) = self.disconnect(&device_id, "stale connection").await {
                warn!("Error disconnecting stale connection {}: {}", device_id, e);
            }
        }
    }
    
    /// 获取统计信息
    pub fn stats(&self) -> ManagerStats {
        ManagerStats {
            active_connections: self.connections.len(),
            connected_devices: self.connected_devices().len(),
            pending_connections: 0, // 简化处理
        }
    }
    
    /// 从引用创建（用于内部转换）
    fn from_arc(manager: &ConnectionManager) -> Self {
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<ConnectionEvent>();
        
        Self {
            config: manager.config.clone(),
            local_device_id: manager.local_device_id,
            connections: manager.connections.clone(),
            conn_id_index: manager.conn_id_index.clone(),
            pending_connections: TokioRwLock::new(HashMap::new()),
            device_addresses: manager.device_addresses.clone(),
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
        }
    }
}

/// 管理器统计信息
#[derive(Debug, Clone)]
pub struct ManagerStats {
    pub active_connections: usize,
    pub connected_devices: usize,
    pub pending_connections: usize,
}

/// 重连调度器
pub struct ReconnectScheduler {
    config: RetryConfig,
    pending: DashMap<DeviceId, JoinHandle<()>>,
}

impl ReconnectScheduler {
    pub fn new(config: RetryConfig) -> Self {
        Self {
            config,
            pending: DashMap::new(),
        }
    }
    
    pub fn schedule<F>(&self, device_id: DeviceId, attempt: u32, task: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        // 取消现有的重连任务
        if let Some((_, handle)) = self.pending.remove(&device_id) {
            handle.abort();
        }
        
        let backoff = self.config.backoff_duration(attempt);
        let handle = tokio::spawn(async move {
            sleep(backoff).await;
            task.await;
        });
        
        self.pending.insert(device_id, handle);
    }
    
    pub fn cancel(&self, device_id: &DeviceId) {
        if let Some((_, handle)) = self.pending.remove(device_id) {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
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
        let (manager, _handle) = ConnectionManager::new(
            ConnectionManagerConfig::default(),
            DeviceId::default(),
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
}
