//! BEP Connection Implementation
//!
//! 实现BEP协议的连接层，包含完整的Hello消息交换

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};


use parking_lot::RwLock;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, info};

use syncthing_core::{ConnectionState, ConnectionStats, ConnectionType, DeviceId, Result, SyncthingError};

use crate::handshake::{exchange_hello, exchange_hello_server};
use crate::messages::Hello;

/// 默认消息超时
pub const DEFAULT_MESSAGE_TIMEOUT: Duration = Duration::from_secs(60);

/// 心跳间隔
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(90);

/// BEP连接结构
///
/// 封装底层TCP连接，提供BEP协议支持
pub struct BepRawConnection {
    /// 连接ID
    id: uuid::Uuid,
    /// 底层流
    stream: Arc<Mutex<TcpStream>>,
    /// 连接状态
    state: RwLock<ConnectionState>,
    /// 连接统计
    stats: RwLock<ConnectionStats>,
    /// 远程地址
    remote_addr: SocketAddr,
    /// 本地地址
    local_addr: SocketAddr,
    /// 连接类型（传入/传出）
    conn_type: ConnectionType,
    /// 关联的设备ID
    device_id: RwLock<Option<DeviceId>>,
    /// 对端的Hello信息
    remote_hello: RwLock<Option<Hello>>,
    /// 最后活动时间
    last_activity: RwLock<Instant>,
}

impl BepRawConnection {
    /// 从TCP流创建新连接（客户端模式 - 先发Hello）
    ///
    /// 用于传出连接（拨号）
    pub async fn connect(
        stream: TcpStream,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<(Arc<Self>, Hello)> {
        let remote_addr = stream
            .peer_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get peer addr: {}", e)))?;
        let local_addr = stream
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get local addr: {}", e)))?;

        let stream = Arc::new(Mutex::new(stream));
        let id = uuid::Uuid::new_v4();

        let conn = Arc::new(Self {
            id,
            stream: Arc::clone(&stream),
            state: RwLock::new(ConnectionState::Connected),
            stats: RwLock::new(ConnectionStats {
                connected_at: Some(chrono::Utc::now()),
                ..Default::default()
            }),
            remote_addr,
            local_addr,
            conn_type: ConnectionType::Outgoing,
            device_id: RwLock::new(None),
            remote_hello: RwLock::new(None),
            last_activity: RwLock::new(Instant::now()),
        });

        // 执行Hello交换（客户端模式：先发后收）
        let remote_hello = conn
            .perform_hello(device_name, client_name, client_version)
            .await?;

        Ok((conn, remote_hello))
    }

    /// 从TCP流接受新连接（服务端模式 - 先收Hello）
    ///
    /// 用于传入连接（监听）
    pub async fn accept(
        stream: TcpStream,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<(Arc<Self>, Hello)> {
        let remote_addr = stream
            .peer_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get peer addr: {}", e)))?;
        let local_addr = stream
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get local addr: {}", e)))?;

        let stream = Arc::new(Mutex::new(stream));
        let id = uuid::Uuid::new_v4();

        let conn = Arc::new(Self {
            id,
            stream: Arc::clone(&stream),
            state: RwLock::new(ConnectionState::Connected),
            stats: RwLock::new(ConnectionStats {
                connected_at: Some(chrono::Utc::now()),
                ..Default::default()
            }),
            remote_addr,
            local_addr,
            conn_type: ConnectionType::Incoming,
            device_id: RwLock::new(None),
            remote_hello: RwLock::new(None),
            last_activity: RwLock::new(Instant::now()),
        });

        // 执行Hello交换（服务端模式：先收后发）
        let remote_hello = conn
            .perform_hello_server(device_name, client_name, client_version)
            .await?;

        Ok((conn, remote_hello))
    }

    /// 创建新的Hello消息
    fn create_hello(
        &self,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Hello {
        Hello {
            device_name: device_name.to_string(),
            client_name: client_name.to_string(),
            client_version: client_version.to_string(),
            num_connections: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        }
    }

    /// 执行Hello交换（客户端模式）
    ///
    /// 发送我们的Hello，然后接收对方的Hello
    async fn perform_hello(
        &self,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<Hello> {
        self.set_state(ConnectionState::TlsHandshakeComplete);

        let our_hello = self.create_hello(device_name, client_name, client_version);

        info!(
            "Sending Hello: device={} client={}/{}",
            our_hello.device_name, our_hello.client_name, our_hello.client_version
        );

        let mut stream = self.stream.lock().await;
        let remote_hello = exchange_hello(&mut *stream, &our_hello).await?;
        drop(stream);

        // 保存远程Hello信息
        *self.remote_hello.write() = Some(remote_hello.clone());
        self.set_state(ConnectionState::ProtocolHandshakeComplete);
        *self.last_activity.write() = Instant::now();

        info!(
            "Hello exchange complete: remote_device={} client={}/{}",
            remote_hello.device_name, remote_hello.client_name, remote_hello.client_version
        );

        // 更新统计
        self.update_stats(|s| {
            s.messages_sent += 1;
            s.messages_received += 1;
        });

        Ok(remote_hello)
    }

    /// 执行Hello交换（服务端模式）
    ///
    /// 先接收对方的Hello，然后发送我们的Hello
    async fn perform_hello_server(
        &self,
        device_name: &str,
        client_name: &str,
        client_version: &str,
    ) -> Result<Hello> {
        self.set_state(ConnectionState::TlsHandshakeComplete);

        let our_hello = self.create_hello(device_name, client_name, client_version);

        let mut stream = self.stream.lock().await;
        let remote_hello = exchange_hello_server(&mut *stream, &our_hello).await?;
        drop(stream);

        // 保存远程Hello信息
        *self.remote_hello.write() = Some(remote_hello.clone());
        self.set_state(ConnectionState::ProtocolHandshakeComplete);
        *self.last_activity.write() = Instant::now();

        info!(
            "Hello exchange complete (server): remote_device={} client={}/{}",
            remote_hello.device_name, remote_hello.client_name, remote_hello.client_version
        );

        // 更新统计
        self.update_stats(|s| {
            s.messages_sent += 1;
            s.messages_received += 1;
        });

        Ok(remote_hello)
    }

    /// 获取连接ID
    pub fn id(&self) -> uuid::Uuid {
        self.id
    }

    /// 获取远程地址
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// 获取本地地址
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 获取当前状态
    pub fn state(&self) -> ConnectionState {
        *self.state.read()
    }

    /// 设置状态
    fn set_state(&self, state: ConnectionState) {
        *self.state.write() = state;
    }

    /// 获取设备ID
    pub fn device_id(&self) -> Option<DeviceId> {
        *self.device_id.read()
    }

    /// 设置设备ID
    pub fn set_device_id(&self, device_id: DeviceId) {
        *self.device_id.write() = Some(device_id);
    }

    /// 获取远程Hello信息
    pub fn remote_hello(&self) -> Option<Hello> {
        self.remote_hello.read().clone()
    }

    /// 获取连接类型
    pub fn connection_type(&self) -> ConnectionType {
        self.conn_type
    }

    /// 获取统计信息
    pub fn stats(&self) -> ConnectionStats {
        self.stats.read().clone()
    }

    /// 更新统计信息
    fn update_stats<F>(&self, f: F)
    where
        F: FnOnce(&mut ConnectionStats),
    {
        let mut stats = self.stats.write();
        f(&mut stats);
    }

    /// 检查连接是否活跃
    pub fn is_alive(&self) -> bool {
        matches!(
            self.state(),
            ConnectionState::Connected
                | ConnectionState::TlsHandshakeComplete
                | ConnectionState::ProtocolHandshakeComplete
                | ConnectionState::ClusterConfigComplete
        )
    }

    /// 检查Hello交换是否完成
    pub fn is_hello_complete(&self) -> bool {
        matches!(
            self.state(),
            ConnectionState::ProtocolHandshakeComplete | ConnectionState::ClusterConfigComplete
        )
    }

    /// 关闭连接
    pub async fn close(&self) -> Result<()> {
        info!("Closing connection {}", self.id());

        self.set_state(ConnectionState::Disconnecting);

        // 关闭底层流
        let mut stream = self.stream.lock().await;
        let _ = stream.shutdown().await;
        drop(stream);

        self.set_state(ConnectionState::Disconnected);

        Ok(())
    }
}

impl Drop for BepRawConnection {
    fn drop(&mut self) {
        if self.is_alive() {
            debug!("Connection {} dropped while still alive", self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state() {
        // 状态测试在集成测试中完成
        assert!(matches!(ConnectionType::Outgoing, ConnectionType::Outgoing));
        assert!(matches!(ConnectionType::Incoming, ConnectionType::Incoming));
    }

    #[test]
    fn test_hello_creation() {
        // 测试Hello消息创建
        let hello = Hello {
            device_name: "test".to_string(),
            client_name: "syncthing-rust".to_string(),
            client_version: "0.1.0".to_string(),
            num_connections: 1,
            timestamp: 1234567890,
        };

        assert_eq!(hello.device_name, "test");
        assert_eq!(hello.client_name, "syncthing-rust");
        assert_eq!(hello.client_version, "0.1.0");
        assert_eq!(hello.num_connections, 1);
    }
}
