//! Syncthing Relay Protocol v1 — 服务端实现
//!
//! 实现 relay 协议的服务端：接受客户端连接、注册设备、配对会话、
//! 双向转发数据。与 Go Syncthing relay 服务器完全互操作。
//!
//! 协议规范: <https://docs.syncthing.net/specs/relay-v1.html>

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Buf, Bytes};
use parking_lot::RwLock;
use rand::RngCore;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, oneshot};
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, trace, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

use super::protocol::{
    self, ConnectRequest, Header, JoinRelayRequest, JoinSessionRequest, MessageType, Ping, Pong,
    Response, SessionInvitation,
};

// ── 常量 ──────────────────────────────────────────────────────

/// 协议模式连接的超时
const PROTOCOL_TIMEOUT: Duration = Duration::from_secs(30);
/// Session 模式连接的超时
const SESSION_TIMEOUT: Duration = Duration::from_secs(30);
/// Ping 间隔
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Ping 超时
const PING_TIMEOUT: Duration = Duration::from_secs(10);
/// 最大并发客户端数
const MAX_CLIENTS: usize = 1000;
/// 会话密钥长度（随机生成）
const SESSION_KEY_LEN: usize = 32;

// ── 内部类型 ──────────────────────────────────────────────────

/// 已注册的 relay 客户端
#[allow(dead_code)]
struct Client {
    device_id: DeviceId,
    /// 协议模式 TLS 连接的写入端
    writer:
        Arc<tokio::sync::Mutex<tokio::io::WriteHalf<tokio_rustls::server::TlsStream<TcpStream>>>>,
    /// 待处理的连接请求（会话配对用，wip）
    pending_connect: HashMap<DeviceId, oneshot::Sender<()>>,
}

/// 待配对的 session
///
/// 配对协议：
/// 1. 第一个 peer 到达 → 存储其 stream + 创建 oneshot tx（供第二个 peer 发 stream 过来），等待 rx
/// 2. 第二个 peer 到达 → 取出存储的 stream + tx，通过 tx 发自己的 stream 给第一个 peer
/// 3. 双方各自拿到对方的 stream，开始双向转发
#[allow(dead_code)]
struct PendingSession {
    from: DeviceId,
    to: DeviceId,
    key: Vec<u8>,
    /// (第一个到达方的 stream, oneshot tx → 第二个到达方通过此 tx 发送自己的 stream)
    first_arrived: Option<(TcpStream, oneshot::Sender<TcpStream>)>,
}

/// Relay 服务器
pub struct RelayServer {
    /// 监听地址（协议模式 + 可选的 WebSocket）
    listen_addr: SocketAddr,
    /// Session 模式监听地址
    session_addr: SocketAddr,
    /// TLS 配置
    tls_config: Arc<crate::tls::SyncthingTlsConfig>,
    /// 已注册客户端（device_id → Client）
    clients: RwLock<HashMap<DeviceId, Arc<Client>>>,
    /// 待配对的 session（key → PendingSession）
    pending_sessions: RwLock<HashMap<Vec<u8>, PendingSession>>,
    /// 关闭信号
    shutdown_tx: RwLock<Option<broadcast::Sender<()>>>,
}

// ── 公共 API ──────────────────────────────────────────────────

impl RelayServer {
    /// 创建新的 relay 服务器
    pub fn new(
        listen_addr: SocketAddr,
        session_addr: SocketAddr,
        tls_config: Arc<crate::tls::SyncthingTlsConfig>,
    ) -> Self {
        Self {
            listen_addr,
            session_addr,
            tls_config,
            clients: RwLock::new(HashMap::new()),
            pending_sessions: RwLock::new(HashMap::new()),
            shutdown_tx: RwLock::new(None),
        }
    }

    /// 启动 relay 服务器
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let mut shutdown_rx_protocol = shutdown_tx.subscribe();
        let mut shutdown_rx_session = shutdown_tx.subscribe();
        *self.shutdown_tx.write() = Some(shutdown_tx);

        // 启动 TLS 协议模式监听器
        let server_config = self
            .tls_config
            .relay_server_config()
            .map_err(|e| SyncthingError::Tls(format!("relay server TLS config: {}", e)))?;
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let protocol_listener = TcpListener::bind(self.listen_addr).await.map_err(|e| {
            SyncthingError::connection(format!("relay protocol bind {}: {}", self.listen_addr, e))
        })?;
        let actual_protocol_addr = protocol_listener
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("relay protocol local_addr: {}", e)))?;
        info!("Relay protocol listener on {}", actual_protocol_addr);

        // 启动 Session 模式监听器
        let session_listener = TcpListener::bind(self.session_addr).await.map_err(|e| {
            SyncthingError::connection(format!("relay session bind {}: {}", self.session_addr, e))
        })?;
        let actual_session_addr = session_listener
            .local_addr()
            .map_err(|e| SyncthingError::connection(format!("relay session local_addr: {}", e)))?;
        info!("Relay session listener on {}", actual_session_addr);

        let self_clients = Arc::clone(&self);
        let self_sessions = Arc::clone(&self);
        let acceptor_clone = acceptor.clone();

        // 协议模式 accept 循环
        let protocol_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx_protocol.recv() => {
                        info!("Relay protocol listener shutting down");
                        break;
                    }
                    result = protocol_listener.accept() => {
                        match result {
                            Ok((stream, peer_addr)) => {
                                debug!("Relay protocol connection from {}", peer_addr);
                                let acceptor = acceptor_clone.clone();
                                let server = Arc::clone(&self_clients);
                                tokio::spawn(async move {
                                    if let Err(e) = Self::handle_protocol_connection(
                                        &server, acceptor, stream, peer_addr,
                                    ).await {
                                        warn!("Protocol connection {} error: {}", peer_addr, e);
                                    }
                                });
                            }
                            Err(e) => {
                                error!("Relay protocol accept error: {}", e);
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }
        });

        // Session 模式 accept 循环
        let session_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx_session.recv() => {
                        info!("Relay session listener shutting down");
                        break;
                    }
                    result = session_listener.accept() => {
                        match result {
                            Ok((stream, peer_addr)) => {
                                debug!("Relay session connection from {}", peer_addr);
                                let server = Arc::clone(&self_sessions);
                                tokio::spawn(async move {
                                    if let Err(e) = Self::handle_session_connection(
                                        &server, stream, peer_addr,
                                    ).await {
                                        warn!("Session connection {} error: {}", peer_addr, e);
                                    }
                                });
                            }
                            Err(e) => {
                                error!("Relay session accept error: {}", e);
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }
        });

        // 等待任务完成
        let _ = tokio::join!(protocol_task, session_task);
        info!("Relay server stopped");
        Ok(())
    }

    /// 优雅关闭
    pub fn shutdown(&self) {
        if let Some(tx) = self.shutdown_tx.write().take() {
            let _ = tx.send(());
        }
    }
}

// ── 协议模式处理 ──────────────────────────────────────────────

impl RelayServer {
    async fn handle_protocol_connection(
        server: &Arc<RelayServer>,
        acceptor: TlsAcceptor,
        stream: TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        // TLS 握手
        let tls_stream = timeout(PROTOCOL_TIMEOUT, acceptor.accept(stream))
            .await
            .map_err(|_| SyncthingError::timeout("relay TLS accept timeout"))?
            .map_err(|e| SyncthingError::Tls(format!("relay TLS accept: {}", e)))?;

        // 从 TLS 证书派生 device ID
        let certs = tls_stream.get_ref().1.peer_certificates();
        let device_id = certs
            .and_then(|certs| certs.first())
            .map(|cert| crate::tls::SyncthingTlsConfig::derive_device_id(cert))
            .transpose()?
            .unwrap_or_else(DeviceId::default);

        let (mut read_half, write_half) = tokio::io::split(tls_stream);
        let writer = Arc::new(tokio::sync::Mutex::new(write_half));

        debug!(
            device = %device_id.short_id(),
            peer = %peer_addr,
            "Relay protocol client connected"
        );

        // 注册客户端 — 先检查容量（无需持有锁跨越 await）
        let is_full = { server.clients.read().len() >= MAX_CLIENTS };
        if is_full {
            let msg = protocol::RelayFull.encode();
            let mut w = writer.lock().await;
            let _ = timeout(PROTOCOL_TIMEOUT, w.write_all(&msg)).await;
            return Err(SyncthingError::config("relay full"));
        }

        {
            let mut clients = server.clients.write();
            clients.insert(
                device_id,
                Arc::new(Client {
                    device_id,
                    writer: Arc::clone(&writer),
                    pending_connect: HashMap::new(),
                }),
            );
        }
        info!(device = %device_id.short_id(), "Client registered (total: {})", server.clients.read().len());

        // 标记离开时清理
        let _guard = ClientGuard {
            server: Arc::clone(server),
            device_id,
        };

        // 启动 Ping 定时器
        let ping_writer = Arc::clone(&writer);
        let ping_device = device_id;
        let ping_server = Arc::clone(server);
        let ping_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(PING_INTERVAL);
            loop {
                interval.tick().await;
                let mut w = ping_writer.lock().await;
                match timeout(PING_TIMEOUT, w.write_all(&Ping.encode())).await {
                    Ok(Ok(_)) => trace!(device = %ping_device.short_id(), "Ping sent"),
                    _ => {
                        debug!(device = %ping_device.short_id(), "Ping failed, client gone");
                        ping_server.remove_client(&ping_device);
                        break;
                    }
                }
            }
        });

        // 读取循环 — 处理 Protocol Mode 消息
        let mut buf = vec![0u8; 65536];
        loop {
            let n = match timeout(PROTOCOL_TIMEOUT, read_half.read(&mut buf)).await {
                Ok(Ok(0)) => {
                    debug!(device = %device_id.short_id(), "Protocol client disconnected");
                    break;
                }
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    warn!(device = %device_id.short_id(), error = %e, "Protocol read error");
                    break;
                }
                Err(_) => {
                    trace!(device = %device_id.short_id(), "Protocol read timeout");
                    continue;
                }
            };

            let mut data = Bytes::from(buf[..n].to_vec());
            while data.remaining() >= Header::SIZE {
                let header = match Header::decode(&mut data) {
                    Some(h) => h,
                    None => break,
                };
                if header.magic != protocol::MAGIC {
                    warn!("Bad magic from {}", device_id.short_id());
                    break;
                }
                let body_len = header.message_length.max(0) as usize;
                if data.remaining() < body_len {
                    break; // 等待更多数据
                }
                let body = data.split_to(body_len);
                let mut body_buf = body;

                match header.message_type {
                    MessageType::Ping => {
                        trace!(device = %device_id.short_id(), "Ping received");
                        let pong = Pong.encode();
                        let mut w = writer.lock().await;
                        let _ = timeout(PROTOCOL_TIMEOUT, w.write_all(&pong)).await;
                    }
                    MessageType::Pong => {
                        trace!(device = %device_id.short_id(), "Pong received");
                    }
                    MessageType::JoinRelayRequest => {
                        let _req = JoinRelayRequest; // 已在上方注册，回复成功
                        let resp = Response::success().encode();
                        let mut w = writer.lock().await;
                        let _ = timeout(PROTOCOL_TIMEOUT, w.write_all(&resp)).await;
                        debug!(device = %device_id.short_id(), "JoinRelay accepted");
                    }
                    MessageType::ConnectRequest => {
                        if let Some(req) = ConnectRequest::decode(&mut body_buf) {
                            Self::handle_connect_request(server, device_id, &req, &writer).await;
                        }
                    }
                    MessageType::Response => {
                        // 客户端不应该发 Response
                        trace!("Unexpected Response from client");
                    }
                    MessageType::SessionInvitation => {
                        // 客户端不应该发 SessionInvitation
                        trace!("Unexpected SessionInvitation from client");
                    }
                    MessageType::JoinSessionRequest | MessageType::RelayFull => {
                        // 这些消息只应在 Session Mode 出现
                        warn!(
                            device = %device_id.short_id(),
                            msg_type = ?header.message_type,
                            "Unexpected message type in protocol mode"
                        );
                    }
                }
            }
        }

        ping_task.abort();
        info!(device = %device_id.short_id(), "Protocol client disconnected");
        Ok(())
    }

    /// 处理连接请求 — 检查目标是否已注册，发送 SessionInvitation
    async fn handle_connect_request(
        server: &Arc<RelayServer>,
        from_device: DeviceId,
        req: &ConnectRequest,
        writer: &Arc<
            tokio::sync::Mutex<tokio::io::WriteHalf<tokio_rustls::server::TlsStream<TcpStream>>>,
        >,
    ) {
        let target_id = match DeviceId::from_bytes(&req.id) {
            Ok(id) => id,
            Err(e) => {
                warn!("Invalid ConnectRequest device ID: {}", e);
                let resp = Response {
                    code: 1,
                    message: format!("invalid device ID: {}", e),
                }
                .encode();
                let mut w = writer.lock().await;
                let _ = timeout(PROTOCOL_TIMEOUT, w.write_all(&resp)).await;
                return;
            }
        };

        info!(
            from = %from_device.short_id(),
            to = %target_id.short_id(),
            "ConnectRequest received"
        );

        // 检查目标客户端是否已注册到此 relay
        let target_client = {
            let clients = server.clients.read();
            clients.get(&target_id).cloned()
        };

        let target_client = match target_client {
            Some(c) => c,
            None => {
                debug!(target = %target_id.short_id(), "ConnectRequest target not connected");
                let resp = Response::not_found().encode();
                let mut w = writer.lock().await;
                let _ = timeout(PROTOCOL_TIMEOUT, w.write_all(&resp)).await;
                return;
            }
        };

        // 生成会话密钥
        let mut key = vec![0u8; SESSION_KEY_LEN];
        rand::thread_rng().fill_bytes(&mut key);

        // 注册 pending session（first_arrived 在第一个 session 连接到达时填充）
        {
            let mut sessions = server.pending_sessions.write();
            sessions.insert(
                key.clone(),
                PendingSession {
                    from: from_device,
                    to: target_id,
                    key: key.clone(),
                    first_arrived: None,
                },
            );
        }

        // 向发起方发送 SessionInvitation（server_socket=true 连接后由发起方决定角色）
        let inv_from = SessionInvitation {
            from: from_device.as_bytes().to_vec(),
            key: key.clone(),
            address: vec![], // 空 = 使用同一 relay IP 的 session 端口
            port: 0,         // 将由客户端的 resolve_session_addr 处理
            server_socket: false,
        };
        {
            let mut w = writer.lock().await;
            let _ = timeout(PROTOCOL_TIMEOUT, w.write_all(&inv_from.encode())).await;
        }

        // 向目标方发送 SessionInvitation（server_socket=true）
        let inv_to = SessionInvitation {
            from: from_device.as_bytes().to_vec(),
            key,
            address: vec![],
            port: 0,
            server_socket: true,
        };
        {
            let mut w = target_client.writer.lock().await;
            let _ = timeout(PROTOCOL_TIMEOUT, w.write_all(&inv_to.encode())).await;
        }

        debug!(
            from = %from_device.short_id(),
            to = %target_id.short_id(),
            "SessionInvitation sent to both peers"
        );
    }

    fn remove_client(&self, device_id: &DeviceId) {
        let mut clients = self.clients.write();
        clients.remove(device_id);
        info!(device = %device_id.short_id(), "Client removed (total: {})", clients.len());
    }
}

// ── Session 模式处理 ──────────────────────────────────────────

impl RelayServer {
    /// 处理 session 模式连接 — 两阶段配对
    ///
    /// Phase 1（第一个到达方）：
    ///   存储 stream + 创建 oneshot tx，等待第二个到达方通过 tx 发来 stream
    /// Phase 2（第二个到达方）：
    ///   取出存储的 stream + tx，通过 tx 发送自己的 stream，获取第一个 stream
    async fn handle_session_connection(
        server: &Arc<RelayServer>,
        mut stream: TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        // 读取 JoinSessionRequest
        let mut read_buf = vec![0u8; 8192];
        let n = timeout(SESSION_TIMEOUT, stream.read(&mut read_buf))
            .await
            .map_err(|_| SyncthingError::timeout("session read timeout"))?
            .map_err(SyncthingError::Io)?;
        if n == 0 {
            return Ok(());
        }
        read_buf.truncate(n);

        let mut data = Bytes::from(read_buf);
        let header = Header::decode(&mut data)
            .ok_or_else(|| SyncthingError::protocol("invalid session header"))?;
        if header.magic != protocol::MAGIC || header.message_type != MessageType::JoinSessionRequest
        {
            return Err(SyncthingError::protocol(
                "expected JoinSessionRequest in session mode",
            ));
        }

        let body_len = header.message_length.max(0) as usize;
        let body = if data.remaining() >= body_len {
            data.split_to(body_len)
        } else {
            data
        };
        let mut body_buf = body;
        let join_req = JoinSessionRequest::decode(&mut body_buf)
            .ok_or_else(|| SyncthingError::protocol("invalid JoinSessionRequest"))?;

        let key = join_req.key;
        debug!(peer = %peer_addr, key_len = key.len(), "Session join request");

        // 两阶段配对：在锁内决定角色，锁外执行 I/O
        enum SessionRole {
            Phase1(oneshot::Receiver<TcpStream>),
            Phase2(TcpStream, TcpStream), // (first_stream, my_stream)
        }

        let role = {
            let mut sessions = server.pending_sessions.write();
            let session = match sessions.get_mut(&key) {
                Some(s) => s,
                None => {
                    warn!(peer = %peer_addr, "Session key not found");
                    return Err(SyncthingError::protocol("session key not found"));
                }
            };

            if let Some((first_stream, _tx_to_first)) = session.first_arrived.take() {
                // Phase 2: 拥有双方 stream
                SessionRole::Phase2(first_stream, stream)
            } else {
                // Phase 1: 存储自己的 stream，等待 Phase 2
                let (tx, rx) = oneshot::channel::<TcpStream>();
                session.first_arrived = Some((stream, tx));
                SessionRole::Phase1(rx)
            }
        }; // sessions 锁在此释放

        match role {
            SessionRole::Phase1(rx) => {
                debug!("First session peer — stored stream, waiting for second");
                match timeout(SESSION_TIMEOUT, rx).await {
                    Ok(Ok(_s)) => {
                        debug!("First peer notified of pairing completion");
                        Ok(())
                    }
                    _ => {
                        server.pending_sessions.write().remove(&key);
                        Err(SyncthingError::timeout(
                            "timed out waiting for second session peer",
                        ))
                    }
                }
            }
            SessionRole::Phase2(mut first_stream, mut my_stream) => {
                let resp = Response::success().encode();
                timeout(SESSION_TIMEOUT, first_stream.write_all(&resp))
                    .await
                    .map_err(|_| SyncthingError::timeout("session response to first peer timeout"))?
                    .map_err(SyncthingError::Io)?;
                timeout(SESSION_TIMEOUT, my_stream.write_all(&resp))
                    .await
                    .map_err(|_| SyncthingError::timeout("session response timeout"))?
                    .map_err(SyncthingError::Io)?;

                info!("Session peers paired — starting bidirectional forward");
                server.pending_sessions.write().remove(&key);
                Self::spawn_bidirectional_forward(first_stream, my_stream);
                Ok(())
            }
        }
    }

    /// 启动双向数据转发：A→B 和 B→A。
    /// 任一方向断开时，自动清理另一个方向。
    fn spawn_bidirectional_forward(a: TcpStream, b: TcpStream) {
        let (mut a_read, mut a_write) = tokio::io::split(a);
        let (mut b_read, mut b_write) = tokio::io::split(b);

        // A → B
        let a_to_b = tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                match a_read.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if b_write.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = b_write.shutdown().await;
        });

        // B → A
        let b_to_a = tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                match b_read.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if a_write.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = a_write.shutdown().await;
        });

        // 任一方向断开时，清理
        tokio::spawn(async move {
            tokio::select! {
                _ = a_to_b => {},
                _ = b_to_a => {},
            }
        });
    }
}

// ── 客户端清理守卫 ────────────────────────────────────────────

struct ClientGuard {
    server: Arc<RelayServer>,
    device_id: DeviceId,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.server.remove_client(&self.device_id);
    }
}

// ── 测试 ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_server_creation() {
        let (cert_pem, key_pem) = crate::tls::generate_certificate("test-relay").unwrap();
        let tls_config =
            Arc::new(crate::tls::SyncthingTlsConfig::from_pem(&cert_pem, &key_pem).unwrap());

        let server = RelayServer::new(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
            tls_config,
        );
        assert_eq!(server.clients.read().len(), 0);
        assert_eq!(server.pending_sessions.read().len(), 0);
    }
}
