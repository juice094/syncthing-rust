//! BEP Session state machine
//!
//! Encapsulates the BEP protocol logic after transport handshake:
//! - ClusterConfig exchange
//! - Initial Index transmission
//! - Steady-state message loop (Ping, Index, IndexUpdate, Request, Response, Close)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tracing::{debug, info, warn};

use syncthing_core::{ConnectionState, DeviceId, Identity, Result, SyncthingError};

use crate::connection::BepConnection;
use crate::metrics;
use crate::protocol::MessageType;

/// Observable events emitted by a BEP session at key state transitions.
#[derive(Debug, Clone)]
pub enum BepSessionEvent {
    /// ClusterConfig exchange completed (both directions).
    ClusterConfigComplete {
        device_id: DeviceId,
        shared_folders: Vec<String>,
    },
    /// Initial Index sent to peer.
    IndexSent {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
    },
    /// Received full Index from peer.
    IndexReceived {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
    },
    /// Received IndexUpdate from peer.
    IndexUpdateReceived {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
    },
    /// Peer requested a block from us (push direction active).
    BlockRequested {
        device_id: DeviceId,
        folder: String,
        name: String,
        offset: i64,
        size: i32,
    },
    /// Heartbeat timeout detected.
    HeartbeatTimeout {
        device_id: DeviceId,
        last_recv_age: Duration,
    },
    /// Peer index changed; completion state should be re-queried.
    PeerSyncState {
        device_id: DeviceId,
        folder: String,
    },
    /// Session ended (clean close or error).
    SessionEnded {
        device_id: DeviceId,
                        reason: String,
    },
}

/// Per-session counters for observability.
#[derive(Debug, Default)]
pub struct BepSessionMetrics {
    pub messages_sent: AtomicU64,
    pub messages_recv: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub bytes_recv: AtomicU64,
    pub blocks_requested: AtomicU64,
    pub blocks_served: AtomicU64,
    pub heartbeat_timeouts: AtomicU64,
    pub errors: AtomicU64,
}

/// Handler trait for BEP session events.
///
/// Implementors provide domain-specific logic (e.g., index generation,
/// block serving) without owning the message loop.
#[async_trait::async_trait]
pub trait BepSessionHandler: Send + Sync {
    /// Generate the local ClusterConfig to send to `device_id`.
    async fn generate_cluster_config(
        &self,
        device_id: DeviceId,
    ) -> Result<bep_protocol::messages::ClusterConfig>;

    /// Generate the initial Index for `folder_id` when talking to `device_id`.
    async fn generate_index(&self, folder_id: &str, device_id: DeviceId) -> Result<syncthing_core::types::Index>;

    /// Remote peer sent a full Index.
    async fn on_index(&self, device_id: DeviceId, index: syncthing_core::types::Index) -> Result<()>;

    /// Remote peer sent an IndexUpdate.
    async fn on_index_update(
        &self,
        device_id: DeviceId,
        update: syncthing_core::types::IndexUpdate,
    ) -> Result<()>;

    /// Remote peer requested a block. Return the raw block bytes or an error code.
    async fn on_block_request(
        &self,
        device_id: DeviceId,
        req: bep_protocol::messages::Request,
    ) -> std::result::Result<Vec<u8>, bep_protocol::messages::ErrorCode>;
}

/// BEP protocol session for a single connection.
pub struct BepSession {
    /// 远程设备身份（抽象层）
    #[allow(dead_code)]
    identity: Arc<dyn Identity>,
    /// 远程设备ID（从 identity 缓存）
    device_id: DeviceId,
    conn: Arc<BepConnection>,
    handler: Arc<dyn BepSessionHandler>,
    pending_responses: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>>,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<BepSessionEvent>>,
    metrics: Arc<BepSessionMetrics>,
    remote_shared_folders: Option<Vec<String>>,
}

impl BepSession {
    /// Create a new session.
    pub fn new(
        identity: Arc<dyn Identity>,
        conn: Arc<BepConnection>,
        handler: Arc<dyn BepSessionHandler>,
        pending_responses: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>>,
    ) -> Self {
        let device_id = identity.device_id();
        Self {
            identity,
            device_id,
            conn,
            handler,
            pending_responses,
            event_tx: None,
            metrics: Arc::new(BepSessionMetrics::default()),
            remote_shared_folders: None,
        }
    }

    /// Create a new session with event subscription.
    pub fn with_events(
        identity: Arc<dyn Identity>,
        conn: Arc<BepConnection>,
        handler: Arc<dyn BepSessionHandler>,
        pending_responses: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>>,
        event_tx: tokio::sync::mpsc::UnboundedSender<BepSessionEvent>,
    ) -> Self {
        let device_id = identity.device_id();
        Self {
            identity,
            device_id,
            conn,
            handler,
            pending_responses,
            event_tx: Some(event_tx),
            metrics: Arc::new(BepSessionMetrics::default()),
            remote_shared_folders: None,
        }
    }

    fn emit(&self, event: BepSessionEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// Return a clone of the per-session metrics arc.
    pub fn metrics(&self) -> Arc<BepSessionMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Run the full BEP session lifecycle.
    pub async fn run(mut self) -> Result<()> {
        // 1. Send ClusterConfig
        let cc = self.handler.generate_cluster_config(self.device_id).await?;
        if let Err(e) = self.conn.send_cluster_config(&cc).await {
            warn!("Failed to send ClusterConfig to {}: {}", self.device_id, e);
            return Err(e);
        }
        info!(
            "Sent ClusterConfig to {} ({} folders)",
            self.device_id,
            cc.folders.len()
        );

        // 2. Wait for remote ClusterConfig
        loop {
            match tokio::time::timeout(Duration::from_secs(10), self.conn.recv_message()).await {
                Ok(Ok((msg_type, payload))) => {
                    match msg_type {
                        MessageType::ClusterConfig => {
                            match bep_protocol::messages::decode_message::<
                                bep_protocol::messages::ClusterConfig,
                            >(&payload)
                            {
                                Ok(remote_cc) => {
                                    info!(
                                        "Received ClusterConfig from {} ({} folders)",
                                        self.device_id,
                                        remote_cc.folders.len()
                                    );
                                    self.conn.set_state(ConnectionState::ClusterConfigComplete);
                                    let remote_shared: Vec<String> = remote_cc.folders.into_iter().map(|f| f.id).collect();
                                    self.emit(BepSessionEvent::ClusterConfigComplete {
                                        device_id: self.device_id,
                                        shared_folders: remote_shared.clone(),
                                    });
                                    // Save remote shared folders for index filtering
                                    self.remote_shared_folders = Some(remote_shared);
                                    break;
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to decode ClusterConfig from {}: {} (payload hex: {})",
                                        self.device_id,
                                        e,
                                        hex::encode(&payload)
                                    );
                                }
                            }
                        }
                        MessageType::Ping => {
                            if let Err(e) = self.conn.send_ping().await {
                                warn!("Failed to send ping reply to {}: {}", self.device_id, e);
                            }
                        }
                        _ => {
                            debug!(
                                "Ignoring message {:?} before ClusterConfig complete",
                                msg_type
                            );
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!("Connection error with {}: {}", self.device_id, e);
                    return Err(e);
                }
                Err(_) => {
                    warn!("Timeout waiting for ClusterConfig from {}", self.device_id);
                    return Err(SyncthingError::timeout(
                        "remote ClusterConfig not received",
                    ));
                }
            }
        }

        // 3. Send Index for each folder shared by BOTH sides
        let my_folder_ids: Vec<String> = cc.folders.into_iter().map(|f| f.id).collect();
        let shared_folder_ids: Vec<String> = match &self.remote_shared_folders {
            Some(remote) => my_folder_ids.into_iter().filter(|id| remote.contains(id)).collect(),
            None => my_folder_ids,
        };
        for folder_id in &shared_folder_ids {
            match self.handler.generate_index(folder_id, self.device_id).await {
                Ok(index) => {
                    let file_count = index.files.len();
                    if let Err(e) = self.conn.send_index(&index).await {
                        warn!(
                            "Failed to send Index for {} to {}: {}",
                            folder_id, self.device_id, e
                        );
                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                    } else {
                        info!(
                            "Sent Index for {} to {} ({} files)",
                            folder_id, self.device_id, file_count
                        );
                        self.emit(BepSessionEvent::IndexSent {
                            device_id: self.device_id,
                            folder: folder_id.clone(),
                            file_count,
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to generate index for {}: {}", folder_id, e);
                }
            }
        }

        // 4. Steady-state message loop
        info!("Entering steady-state BEP loop for {}", self.device_id);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(90));
        let mut last_recv = Instant::now();
        #[allow(unused_assignments)]
        let mut session_end_reason = String::new();
        loop {
            tokio::select! {
                result = self.conn.recv_message() => {
                    let latency = last_recv.elapsed();
                    match result {
                        Ok((msg_type, payload)) => {
                            let payload_len = payload.len() as u64;
                            metrics::global().record_bep_message_recv(
                                self.device_id.to_string(),
                                &format!("{:?}", msg_type),
                                latency,
                                payload_len,
                            );
                            self.metrics.messages_recv.fetch_add(1, Ordering::Relaxed);
                            self.metrics.bytes_recv.fetch_add(payload_len, Ordering::Relaxed);
                            last_recv = Instant::now();
                            if let Err(e) = self.handle_message(msg_type, payload).await {
                                warn!("BEP session loop error for {}: {}", self.device_id, e);
                                session_end_reason = format!("handle_message error: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("BEP session loop error for {}: {}", self.device_id, e);
                            session_end_reason = format!("recv error: {}", e);
                            break;
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    let idle = last_recv.elapsed();
                    if idle > Duration::from_secs(270) {
                        warn!("Heartbeat timeout for {} (idle {:?})", self.device_id, idle);
                        self.metrics.heartbeat_timeouts.fetch_add(1, Ordering::Relaxed);
                        self.emit(BepSessionEvent::HeartbeatTimeout {
                            device_id: self.device_id,
                            last_recv_age: idle,
                        });
                        session_end_reason = format!("heartbeat timeout (idle {:?})", idle);
                        break;
                    }
                    if let Err(e) = self.conn.send_ping().await {
                        warn!("Failed to send ping to {}: {}", self.device_id, e);
                        session_end_reason = format!("ping send error: {}", e);
                        break;
                    }
                    self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        self.emit(BepSessionEvent::SessionEnded {
            device_id: self.device_id,
            reason: session_end_reason,
        });

        Ok(())
    }

    async fn handle_message(&self, msg_type: MessageType, payload: bytes::Bytes) -> Result<()> {
        match msg_type {
            MessageType::Ping => {
                self.conn.send_ping().await?;
                self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);
            }
            MessageType::Index => {
                match bep_protocol::messages::decode_message::<bep_protocol::messages::Index>(&payload)
                {
                    Ok(wire_index) => {
                        let file_count = wire_index.files.len();
                        let folder = wire_index.folder.clone();
                        let index: syncthing_core::types::Index = wire_index.into();
                        self.emit(BepSessionEvent::IndexReceived {
                            device_id: self.device_id,
                            folder: folder.clone(),
                            file_count,
                        });
                        self.emit(BepSessionEvent::PeerSyncState {
                            device_id: self.device_id,
                            folder: folder.clone(),
                        });
                        if let Err(e) = self.handler.on_index(self.device_id, index).await {
                            warn!("Failed to handle Index from {}: {}", self.device_id, e);
                            self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to decode Index from {}: {}", self.device_id, e);
                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            MessageType::IndexUpdate => {
                match bep_protocol::messages::decode_message::<
                    bep_protocol::messages::IndexUpdate,
                >(&payload)
                {
                    Ok(wire_update) => {
                        let file_count = wire_update.files.len();
                        let folder = wire_update.folder.clone();
                        let update: syncthing_core::types::IndexUpdate = wire_update.into();
                        self.emit(BepSessionEvent::IndexUpdateReceived {
                            device_id: self.device_id,
                            folder: folder.clone(),
                            file_count,
                        });
                        self.emit(BepSessionEvent::PeerSyncState {
                            device_id: self.device_id,
                            folder: folder.clone(),
                        });
                        if let Err(e) = self.handler.on_index_update(self.device_id, update).await {
                            warn!(
                                "Failed to handle IndexUpdate from {}: {}",
                                self.device_id, e
                            );
                            self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to decode IndexUpdate from {}: {}",
                            self.device_id, e
                        );
                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            MessageType::Request => {
                match bep_protocol::messages::decode_message::<
                    bep_protocol::messages::Request,
                >(&payload)
                {
                    Ok(req) => {
                        self.metrics.blocks_requested.fetch_add(1, Ordering::Relaxed);
                        self.emit(BepSessionEvent::BlockRequested {
                            device_id: self.device_id,
                            folder: req.folder.clone(),
                            name: req.name.clone(),
                            offset: req.offset,
                            size: req.size,
                        });
                        match self.handler.on_block_request(self.device_id, req.clone()).await {
                            Ok(data) => {
                                let resp = bep_protocol::messages::Response {
                                    id: req.id,
                                    data: data.clone(),
                                    code: bep_protocol::messages::ErrorCode::NoError as i32,
                                };
                                match bep_protocol::messages::encode_message(&resp) {
                                    Ok(payload) => {
                                        let payload_len = payload.len() as u64;
                                        if let Err(e) = self
                                            .conn
                                            .send_message(MessageType::Response, payload)
                                            .await
                                        {
                                            warn!(
                                                "Failed to send Response to {}: {}",
                                                self.device_id, e
                                            );
                                            self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                        } else {
                                            self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);
                                            self.metrics.bytes_sent.fetch_add(payload_len, Ordering::Relaxed);
                                            self.metrics.blocks_served.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Failed to encode Response for {}: {}",
                                            self.device_id, e
                                        );
                                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            Err(code) => {
                                warn!("Block request from {} failed: {:?}", self.device_id, code);
                                let resp = bep_protocol::messages::Response {
                                    id: req.id,
                                    data: vec![],
                                    code: code as i32,
                                };
                                if let Ok(payload) = bep_protocol::messages::encode_message(&resp)
                                {
                                    if let Err(e) = self
                                        .conn
                                        .send_message(MessageType::Response, payload)
                                        .await
                                    {
                                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                        warn!("Failed to send error Response to {}: {}", self.device_id, e);
                                    } else {
                                        self.metrics.messages_sent.fetch_add(1, Ordering::Relaxed);
                                    }
                                } else {
                                    self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to decode Request from {}: {}", self.device_id, e);
                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            MessageType::Response => {
                match bep_protocol::messages::decode_message::<
                    bep_protocol::messages::Response,
                >(&payload)
                {
                    Ok(resp) => {
                        info!(
                            "Received Response id={} code={} data_len={} from {}",
                            resp.id,
                            resp.code,
                            resp.data.len(),
                            self.device_id
                        );
                        if let Some((_, tx)) = self.pending_responses.remove(&resp.id) {
                            let _ = tx.send(resp);
                        } else {
                            warn!(
                                "Received unmatched Response id={} from {}",
                                resp.id, self.device_id
                            );
                            self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to decode Response from {}: {}", self.device_id, e);
                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            MessageType::Close => {
                info!("Received Close from {}", self.device_id);
                return Err(SyncthingError::ConnectionClosed);
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::ConnectionType;

    struct MockHandler {
        index_calls: tokio::sync::Mutex<Vec<(String, DeviceId)>>,
    }

    impl MockHandler {
        fn new() -> Self {
            Self {
                index_calls: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl BepSessionHandler for MockHandler {
        async fn generate_cluster_config(
            &self,
            _device_id: DeviceId,
        ) -> Result<bep_protocol::messages::ClusterConfig> {
            Ok(bep_protocol::messages::ClusterConfig {
                folders: vec![bep_protocol::messages::WireFolder {
                    id: "test-folder".to_string(),
                    label: String::new(),
                    r#type: 0,
                    stop_reason: 0,
                    devices: vec![],
                }],
                secondary: false,
            })
        }

        async fn generate_index(
            &self,
            folder_id: &str,
            device_id: DeviceId,
        ) -> Result<syncthing_core::types::Index> {
            self.index_calls.lock().await.push((folder_id.to_string(), device_id));
            Ok(syncthing_core::types::Index {
                folder: folder_id.to_string(),
                files: vec![],
            })
        }

        async fn on_index(
            &self,
            _device_id: DeviceId,
            _index: syncthing_core::types::Index,
        ) -> Result<()> {
            Ok(())
        }

        async fn on_index_update(
            &self,
            _device_id: DeviceId,
            _update: syncthing_core::types::IndexUpdate,
        ) -> Result<()> {
            Ok(())
        }

        async fn on_block_request(
            &self,
            _device_id: DeviceId,
            _req: bep_protocol::messages::Request,
        ) -> std::result::Result<Vec<u8>, bep_protocol::messages::ErrorCode> {
            Ok(vec![1, 2, 3])
        }
    }

    #[tokio::test]
    async fn test_session_ping_pong() {
        let (pipe_a, pipe_b) = syncthing_test_utils::memory_pipe_pair(4096);
        let (tx_a, _rx_a) = tokio::sync::mpsc::unbounded_channel();
        let (tx_b, _rx_b) = tokio::sync::mpsc::unbounded_channel();

        let conn_a = BepConnection::new(Box::new(pipe_a), ConnectionType::Outgoing, tx_a)
            .await
            .unwrap();
        let conn_b = BepConnection::new(Box::new(pipe_b), ConnectionType::Incoming, tx_b)
            .await
            .unwrap();

        let device_id = DeviceId::default();
        let handler = Arc::new(MockHandler::new());
        let pending: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>> =
            Arc::new(DashMap::new());

        let session = BepSession::new(Arc::new(syncthing_core::DeviceIdentity::new(device_id)), Arc::clone(&conn_a), handler, pending);
        let handle = tokio::spawn(session.run());

        // Wait for ClusterConfig from session side
        let (msg_type, _) = conn_b.recv_message().await.unwrap();
        assert_eq!(msg_type, MessageType::ClusterConfig);

        // Reply with ClusterConfig
        let reply_cc = bep_protocol::messages::ClusterConfig {
            folders: vec![bep_protocol::messages::WireFolder {
                id: "test-folder".to_string(),
                label: vec![],
                r#type: 0,
                stop_reason: 0,
                devices: vec![],
            }],
            secondary: false,
        };
        let payload = bep_protocol::messages::encode_message(&reply_cc).unwrap();
        conn_b.send_message(MessageType::ClusterConfig, payload).await.unwrap();

        // Wait for Index
        let (msg_type, _) = conn_b.recv_message().await.unwrap();
        assert_eq!(msg_type, MessageType::Index);

        // Send a Ping
        conn_b.send_ping().await.unwrap();

        // Should receive Ping back
        let (msg_type, _) = conn_b.recv_message().await.unwrap();
        assert_eq!(msg_type, MessageType::Ping);

        // Clean shutdown
        conn_b.close().await.ok();
        handle.abort();
    }

    #[tokio::test]
    async fn test_session_block_request_response() {
        let (pipe_a, pipe_b) = syncthing_test_utils::memory_pipe_pair(4096);
        let (tx_a, _rx_a) = tokio::sync::mpsc::unbounded_channel();
        let (tx_b, _rx_b) = tokio::sync::mpsc::unbounded_channel();

        let conn_a = BepConnection::new(Box::new(pipe_a), ConnectionType::Outgoing, tx_a)
            .await
            .unwrap();
        let conn_b = BepConnection::new(Box::new(pipe_b), ConnectionType::Incoming, tx_b)
            .await
            .unwrap();

        let device_id = DeviceId::default();
        let handler = Arc::new(MockHandler::new());
        let pending: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>> =
            Arc::new(DashMap::new());

        let session = BepSession::new(Arc::new(syncthing_core::DeviceIdentity::new(device_id)), Arc::clone(&conn_a), handler, Arc::clone(&pending));
        let handle = tokio::spawn(session.run());

        // Handshake: ClusterConfig -> ClusterConfig -> Index
        let (msg_type, _) = conn_b.recv_message().await.unwrap();
        assert_eq!(msg_type, MessageType::ClusterConfig);

        let reply_cc = bep_protocol::messages::ClusterConfig {
            folders: vec![bep_protocol::messages::WireFolder {
                id: "test-folder".to_string(),
                label: vec![],
                r#type: 0,
                stop_reason: 0,
                devices: vec![],
            }],
            secondary: false,
        };
        let payload = bep_protocol::messages::encode_message(&reply_cc).unwrap();
        conn_b.send_message(MessageType::ClusterConfig, payload).await.unwrap();

        let (msg_type, _) = conn_b.recv_message().await.unwrap();
        assert_eq!(msg_type, MessageType::Index);

        // Send a Request from B side
        let req = bep_protocol::messages::Request {
            id: 42,
            folder: "test".to_string(),
            name: "file.txt".to_string(),
            offset: 0,
            size: 3,
            hash: vec![],
            from_temporary: false,
            block_no: 0,
        };
        let req_payload = bep_protocol::messages::encode_message(&req).unwrap();
        conn_b.send_message(MessageType::Request, req_payload).await.unwrap();

        // Should receive Response with mock data [1, 2, 3]
        let (msg_type, resp_payload) = conn_b.recv_message().await.unwrap();
        assert_eq!(msg_type, MessageType::Response);
        let resp = bep_protocol::messages::decode_message::<
            bep_protocol::messages::Response,
        >(&resp_payload)
        .unwrap();
        assert_eq!(resp.id, 42);
        assert_eq!(resp.data, vec![1, 2, 3]);
        assert_eq!(resp.code, bep_protocol::messages::ErrorCode::NoError as i32);

        conn_b.close().await.ok();
        handle.abort();
    }

    #[tokio::test]
    async fn test_session_events_and_metrics() {
        let (pipe_a, pipe_b) = syncthing_test_utils::memory_pipe_pair(4096);
        let (tx_a, _rx_a) = tokio::sync::mpsc::unbounded_channel();
        let (tx_b, _rx_b) = tokio::sync::mpsc::unbounded_channel();

        let conn_a = BepConnection::new(Box::new(pipe_a), ConnectionType::Outgoing, tx_a)
            .await
            .unwrap();
        let conn_b = BepConnection::new(Box::new(pipe_b), ConnectionType::Incoming, tx_b)
            .await
            .unwrap();

        let device_id = DeviceId::default();
        let handler = Arc::new(MockHandler::new());
        let pending: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>> =
            Arc::new(DashMap::new());

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BepSessionEvent>();
        let session = BepSession::with_events(Arc::new(syncthing_core::DeviceIdentity::new(device_id)), Arc::clone(&conn_a), handler, Arc::clone(&pending), event_tx);
        let metrics = session.metrics();
        let handle = tokio::spawn(session.run());

        // 1. Expect ClusterConfig from session side
        let (msg_type, _) = conn_b.recv_message().await.unwrap();
        assert_eq!(msg_type, MessageType::ClusterConfig);

        // 2. Reply with ClusterConfig
        let reply_cc = bep_protocol::messages::ClusterConfig {
            folders: vec![bep_protocol::messages::WireFolder {
                id: "test-folder".to_string(),
                label: vec![],
                r#type: 0,
                stop_reason: 0,
                devices: vec![],
            }],
            secondary: false,
        };
        let payload = bep_protocol::messages::encode_message(&reply_cc).unwrap();
        conn_b.send_message(MessageType::ClusterConfig, payload).await.unwrap();

        // Wait for ClusterConfigComplete event
        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(event, BepSessionEvent::ClusterConfigComplete { .. }));

        // 3. Expect Index from session side
        let (msg_type, _) = conn_b.recv_message().await.unwrap();
        assert_eq!(msg_type, MessageType::Index);

        // Wait for IndexSent event
        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(event, BepSessionEvent::IndexSent { folder, .. } if folder == "test-folder"));

        // 4. Send an Index from B side -> should trigger IndexReceived
        let index = bep_protocol::messages::Index {
            folder: "test-folder".to_string(),
            files: vec![bep_protocol::messages::WireFileInfo {
                name: "hello.txt".to_string(),
                ..Default::default()
            }],
            last_sequence: 0,
        };
        let idx_payload = bep_protocol::messages::encode_message(&index).unwrap();
        conn_b.send_message(MessageType::Index, idx_payload).await.unwrap();

        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(event, BepSessionEvent::IndexReceived { folder, file_count: 1, .. } if folder == "test-folder"));

        // Consume the PeerSyncState event emitted right after IndexReceived
        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(event, BepSessionEvent::PeerSyncState { folder, .. } if folder == "test-folder"));

        // 5. Send a Request -> should trigger BlockRequested + Response
        let req = bep_protocol::messages::Request {
            id: 7,
            folder: "test-folder".to_string(),
            name: "hello.txt".to_string(),
            offset: 0,
            size: 3,
            hash: vec![],
            from_temporary: false,
            block_no: 0,
        };
        let req_payload = bep_protocol::messages::encode_message(&req).unwrap();
        conn_b.send_message(MessageType::Request, req_payload).await.unwrap();

        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(event, BepSessionEvent::BlockRequested { folder, name, size: 3, .. }
            if folder == "test-folder" && name == "hello.txt"));

        // Receive Response (skip stray Ping messages that may arrive from heartbeat)
        let (msg_type, _) = loop {
            let (msg_type, payload) = conn_b.recv_message().await.unwrap();
            if msg_type != MessageType::Ping {
                break (msg_type, payload);
            }
        };
        assert_eq!(msg_type, MessageType::Response);

        // 6. Verify metrics (small yield to let async counters settle)
        tokio::time::sleep(Duration::from_millis(50)).await;
        let recv = metrics.messages_recv.load(Ordering::Relaxed);
        let sent = metrics.messages_sent.load(Ordering::Relaxed);
        assert!(recv >= 2, "expected at least 2 messages_recv, got {}", recv); // Index + Request
        assert_eq!(metrics.blocks_requested.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.blocks_served.load(Ordering::Relaxed), 1);
        assert!(sent >= 2, "expected at least 2 messages_sent, got {}", sent); // Ping reply + Response

        conn_b.close().await.ok();
        handle.abort();
    }
}
