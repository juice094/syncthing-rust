//! TCP 传输实现
//!
//! 实现TCP监听器和拨号器
//! 参考: syncthing/lib/connections/tcp_listen.go, tcp_dial.go

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener as TokioTcpListener, TcpStream};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tracing::{debug, error, info, trace, warn};

use syncthing_core::{ConnectionType, Result, RetryConfig, SyncthingError};

use crate::connection::{BepConnection, TcpBiStream};
use crate::manager::ConnectionManagerHandle;
use crate::tls::SyncthingTlsConfig;
use crate::metrics;

/// 默认TCP端口
pub const DEFAULT_TCP_PORT: u16 = 22000;

/// TCP连接超时
pub const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// TLS握手超时
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 协议握手超时
pub const PROTOCOL_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// TCP 监听器
pub struct SyncthingTcpListener {
    /// 绑定地址
    #[allow(dead_code)]
    bind_addr: SocketAddr,
    /// 底层TCP监听器
    inner: Option<TokioTcpListener>,
    /// 连接管理器句柄
    manager: ConnectionManagerHandle,
    /// 本地设备ID（用于Hello消息）
    local_device_id: syncthing_core::DeviceId,
    /// 设备名称
    device_name: String,
    /// TLS 配置
    tls_config: Arc<SyncthingTlsConfig>,
    /// 运行标志
    running: bool,
    /// 关闭信号发送端
    shutdown_tx: Option<tokio::sync::mpsc::Sender<()>>,
}

impl SyncthingTcpListener {
    /// 创建新的TCP监听器
    pub fn new(
        bind_addr: SocketAddr,
        manager: ConnectionManagerHandle,
        local_device_id: syncthing_core::DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Self {
        Self {
            bind_addr,
            inner: None,
            manager,
            local_device_id,
            device_name,
            tls_config,
            running: false,
            shutdown_tx: None,
        }
    }
    
    /// 绑定并开始监听
    pub async fn bind(&mut self) -> Result<SocketAddr> {
        let listener = TokioTcpListener::bind(self.bind_addr).await
            .map_err(|e| SyncthingError::connection(format!("failed to bind to {}: {}", self.bind_addr, e)))?;
        
        let actual_addr = listener.local_addr()
            .map_err(|e| SyncthingError::connection(format!("failed to get local addr: {}", e)))?;
        
        info!("TCP listener bound to {}", actual_addr);
        
        self.inner = Some(listener);
        self.bind_addr = actual_addr;
        
        Ok(actual_addr)
    }
    
    /// 启动接受循环
    pub async fn run(mut self) -> Result<()> {
        let listener = self.inner.take()
            .ok_or_else(|| SyncthingError::connection("listener not bound"))?;
        
        self.running = true;
        info!("TCP listener started on {}", self.bind_addr);
        
        // 创建关闭通道
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);
        
        // 监听循环（使用 tokio::select! 支持 graceful shutdown）
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("TCP listener received shutdown signal");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            debug!("Incoming TCP connection from {}", peer_addr);
                            
                            // 处理新连接
                            let manager = self.manager.clone();
                            let local_device_id = self.local_device_id;
                            let device_name = self.device_name.clone();
                            let tls_config = Arc::clone(&self.tls_config);
                            
                            tokio::spawn(async move {
                                if let Err(e) = Self::handle_incoming(
                                    stream,
                                    peer_addr,
                                    manager,
                                    local_device_id,
                                    device_name,
                                    tls_config,
                                ).await {
                                    warn!("Failed to handle incoming connection from {}: {}", peer_addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("TCP accept error: {}", e);
                            // 短暂暂停以避免CPU飙升
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }
        
        info!("TCP listener stopped");
        Ok(())
    }
    
    /// 停止监听器
    pub fn stop(&mut self) {
        self.running = false;
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
        }
        info!("TCP listener stopping...");
    }
    
    /// 处理传入连接
    async fn handle_incoming(
        stream: TcpStream,
        _peer_addr: SocketAddr,
        manager: ConnectionManagerHandle,
        _local_device_id: syncthing_core::DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Result<()> {
        // 设置TCP选项
        Self::configure_stream(&stream)?;
        
        // 服务端 TLS 握手
        let tls_start = std::time::Instant::now();
        let (tls_stream, device_id) = crate::tls::accept_tls_stream(stream, &tls_config).await?;
        metrics::global().record_tls_handshake(tls_start.elapsed());
        
        debug!("Server TLS handshake completed: peer_device_id={}", device_id);
        
        // BEP Hello 交换
        let mut tls_stream = tls_stream;
        let _remote_hello = crate::handshaker::BepHandshaker::server_handshake(
            &mut tls_stream,
            &device_name,
        ).await?;
        
        // 创建BEP连接（先不关联设备ID）
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();

        let conn = BepConnection::new(
            Box::new(TcpBiStream::Server(tls_stream)),
            ConnectionType::Incoming,
            event_tx,
        ).await?;

        // 获取连接ID
        let conn_id = conn.id();

        // 设置设备ID并注册到管理器
        conn.set_device_id(device_id);
        manager.register_connection(device_id, conn).await?;

        info!("Incoming connection {} handled successfully", conn_id);
        
        Ok(())
    }
    
    /// 配置TCP流选项
    fn configure_stream(stream: &TcpStream) -> Result<()> {
        // 设置TCP_NODELAY（禁用Nagle算法）
        stream.set_nodelay(true)
            .map_err(|e| SyncthingError::connection(format!("failed to set TCP_NODELAY: {}", e)))?;
        
        Ok(())
    }
}

/// 建立单个TCP连接并完成BEP握手
pub async fn connect_bep(
    addr: SocketAddr,
    target_device: syncthing_core::DeviceId,
    _local_device_id: syncthing_core::DeviceId,
    device_name: &str,
    tls_config: &Arc<SyncthingTlsConfig>,
) -> Result<Arc<BepConnection>> {
    trace!("Dialing {} for device {}", addr, target_device);
    
    // 建立TCP连接
    let stream = timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| SyncthingError::timeout("TCP connect timeout"))?
        .map_err(|e| SyncthingError::connection(format!("TCP connect failed: {}", e)))?;
    
    debug!("TCP connected to {}", addr);
    
    // 配置TCP选项
    SyncthingTcpListener::configure_stream(&stream)?;
    
    // 客户端 TLS 握手
    let tls_start = std::time::Instant::now();
    let client_config = tls_config.client_config()
        .map_err(|e| SyncthingError::Tls(format!("failed to create client config: {}", e)))?;
    let connector = TlsConnector::from(Arc::new(client_config));
    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("syncthing")
        .map_err(|_| SyncthingError::Tls("invalid server name".to_string()))?;
    let tls_stream = timeout(
        TLS_HANDSHAKE_TIMEOUT,
        connector.connect(server_name, stream)
    ).await
        .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
        .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;
    metrics::global().record_tls_handshake(tls_start.elapsed());
    
    debug!("Client TLS handshake completed");
    
    // BEP Hello 交换
    let mut tls_stream = tls_stream;
    let _remote_hello = crate::handshaker::BepHandshaker::client_handshake(
        &mut tls_stream,
        device_name,
    ).await?;
    
    // 创建BEP连接
    let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
    
    let conn = BepConnection::new(
        Box::new(TcpBiStream::Client(tls_stream)),
        ConnectionType::Outgoing,
        event_tx,
    ).await?;
    
    // 设置目标设备ID
    conn.set_device_id(target_device);

    // 更新状态
    conn.set_state(syncthing_core::ConnectionState::ProtocolHandshakeComplete);
    
    info!("Connection {} established to {}", conn.id(), addr);
    
    Ok(conn)
}

/// TCP 拨号器
pub struct TcpDialer {
    /// 连接管理器句柄
    manager: ConnectionManagerHandle,
    /// 本地设备ID
    local_device_id: syncthing_core::DeviceId,
    /// 设备名称
    device_name: String,
    /// 重试配置
    retry_config: RetryConfig,
    /// TLS 配置
    tls_config: Arc<SyncthingTlsConfig>,
}

impl TcpDialer {
    /// 创建新的TCP拨号器
    pub fn new(
        manager: ConnectionManagerHandle,
        local_device_id: syncthing_core::DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Self {
        Self {
            manager,
            local_device_id,
            device_name,
            retry_config: RetryConfig::default(),
            tls_config,
        }
    }
    
    /// 设置重试配置
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }
    
    /// 拨号到指定地址
    pub async fn dial(&self, addr: SocketAddr, target_device: syncthing_core::DeviceId) -> Result<Arc<BepConnection>> {
        let mut last_error = None;
        
        for attempt in 0..=self.retry_config.max_retries {
            match self.dial_once(addr, target_device).await {
                Ok(conn) => {
                    info!("Successfully connected to {} (device: {})", addr, target_device);
                    return Ok(conn);
                }
                Err(e) => {
                    warn!("Dial attempt {} to {} failed: {}", attempt + 1, addr, e);
                    last_error = Some(e);
                    
                    if attempt < self.retry_config.max_retries {
                        let backoff = self.retry_config.backoff_duration(attempt);
                        debug!("Retrying in {:?}", backoff);
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }
        
        Err(last_error.unwrap_or_else(|| SyncthingError::connection("all dial attempts failed")))
    }
    
    /// 单次拨号尝试
    async fn dial_once(&self, addr: SocketAddr, target_device: syncthing_core::DeviceId) -> Result<Arc<BepConnection>> {
        connect_bep(addr, target_device, self.local_device_id, &self.device_name, &self.tls_config).await
    }
    
    /// 拨号并注册到管理器
    pub async fn dial_and_register(&self, addr: SocketAddr, target_device: syncthing_core::DeviceId) -> Result<()> {
        let conn = self.dial(addr, target_device).await?;
        
        // 注册到管理器
        self.manager.register_connection(target_device, conn).await?;
        
        Ok(())
    }
}

/// TCP 传输层（组合监听和拨号）
pub struct TcpTransport {
    /// 监听器
    listener: Option<SyncthingTcpListener>,
    /// 拨号器
    dialer: TcpDialer,
    /// 绑定地址
    #[allow(dead_code)]
    bind_addr: SocketAddr,
}

impl TcpTransport {
    /// 创建新的TCP传输层
    pub fn new(
        bind_addr: SocketAddr,
        manager: ConnectionManagerHandle,
        local_device_id: syncthing_core::DeviceId,
        device_name: String,
        tls_config: Arc<SyncthingTlsConfig>,
    ) -> Self {
        let dialer = TcpDialer::new(
            manager.clone(),
            local_device_id,
            device_name.clone(),
            Arc::clone(&tls_config),
        );
        
        let listener = SyncthingTcpListener::new(
            bind_addr,
            manager,
            local_device_id,
            device_name,
            tls_config,
        );
        
        Self {
            listener: Some(listener),
            dialer,
            bind_addr,
        }
    }
    
    /// 启动传输层
    pub async fn start(&mut self) -> Result<SocketAddr> {
        // 绑定监听器
        let addr = self.listener.as_mut().unwrap().bind().await?;
        
        // 启动监听循环
        let listener = self.listener.take().unwrap();
        tokio::spawn(async move {
            if let Err(e) = listener.run().await {
                error!("TCP listener error: {}", e);
            }
        });
        
        Ok(addr)
    }
    
    /// 获取拨号器引用
    pub fn dialer(&self) -> &TcpDialer {
        &self.dialer
    }
    
    /// 创建到指定设备的连接
    pub async fn connect(&self, addr: SocketAddr, target_device: syncthing_core::DeviceId) -> Result<Arc<BepConnection>> {
        self.dialer.dial(addr, target_device).await
    }
}

/// 地址解析工具
pub mod addr {
    use super::*;
    
    /// 解析地址字符串
    pub fn parse_addr(addr_str: &str) -> Result<SocketAddr> {
        addr_str.parse()
            .map_err(|e| SyncthingError::config(format!("invalid address '{}': {}", addr_str, e)))
    }
    
    /// 解析多个地址
    pub fn parse_addrs(addr_strs: &[&str]) -> Result<Vec<SocketAddr>> {
        addr_strs.iter()
            .map(|s| parse_addr(s))
            .collect()
    }
    
    /// 构建默认监听地址
    pub fn default_listen_addr() -> SocketAddr {
        SocketAddr::from(([0, 0, 0, 0], DEFAULT_TCP_PORT))
    }
    
    /// 构建本地监听地址（仅本地）
    pub fn localhost_listen_addr() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], DEFAULT_TCP_PORT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::DeviceId;
    
    #[test]
    fn test_default_port() {
        assert_eq!(DEFAULT_TCP_PORT, 22000);
    }
    
    #[test]
    fn test_addr_parse() {
        let addr = addr::parse_addr("127.0.0.1:22000").unwrap();
        assert_eq!(addr.port(), 22000);
    }
    
    #[tokio::test]
    async fn test_tls_hello_exchange() {
        // Generate TLS configs for server and client
        let (server_cert, server_key) = crate::tls::generate_certificate("server").unwrap();
        let server_tls = Arc::new(crate::tls::SyncthingTlsConfig::from_pem(&server_cert, &server_key).unwrap());
        
        let (client_cert, client_key) = crate::tls::generate_certificate("client").unwrap();
        let client_tls = Arc::new(crate::tls::SyncthingTlsConfig::from_pem(&client_cert, &client_key).unwrap());
        
        // Start mock server
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_tls_clone = Arc::clone(&server_tls);
        
        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let server_config = server_tls_clone.server_config().unwrap();
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
            let mut tls_stream = acceptor.accept(stream).await.unwrap();
            
            // Server BEP handshake
            let hello = crate::handshaker::BepHandshaker::server_handshake(
                &mut tls_stream,
                "server-device",
            ).await.unwrap();
            
            hello
        });
        
        // Client connects using connect_bep
        let result = connect_bep(addr, DeviceId::default(), DeviceId::default(), "test-device", &client_tls).await;
        assert!(result.is_ok(), "connect_bep failed: {:?}", result.err());
        
        // Verify server received the correct protobuf Hello
        let received_hello = server_handle.await.unwrap();
        assert_eq!(received_hello.device_name, "test-device");
        assert_eq!(received_hello.client_name, "syncthing");
    }
}
