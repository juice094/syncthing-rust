//! BEP Session state machine
//!
//! Encapsulates the BEP protocol logic after transport handshake:
//! - ClusterConfig exchange
//! - Initial Index transmission
//! - Steady-state message loop (Ping, Index, IndexUpdate, Request, Response, Close)

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tracing::{debug, info, warn};

use syncthing_core::{ConnectionState, DeviceId, Result, SyncthingError};

use crate::connection::BepConnection;
use crate::metrics;
use crate::protocol::MessageType;

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
    device_id: DeviceId,
    conn: Arc<BepConnection>,
    handler: Arc<dyn BepSessionHandler>,
    pending_responses: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>>,
}

impl BepSession {
    /// Create a new session.
    pub fn new(
        device_id: DeviceId,
        conn: Arc<BepConnection>,
        handler: Arc<dyn BepSessionHandler>,
        pending_responses: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>>,
    ) -> Self {
        Self {
            device_id,
            conn,
            handler,
            pending_responses,
        }
    }

    /// Run the full BEP session lifecycle.
    pub async fn run(self) -> Result<()> {
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
        let mut remote_cc_received = false;
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
                                    remote_cc_received = true;
                                    self.conn.set_state(ConnectionState::ClusterConfigComplete);
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

        if !remote_cc_received {
            return Ok(());
        }

        // 3. Send Index for each shared folder
        let shared_folder_ids: Vec<String> = cc.folders.into_iter().map(|f| f.id).collect();
        for folder_id in &shared_folder_ids {
            match self.handler.generate_index(folder_id, self.device_id).await {
                Ok(index) => {
                    if let Err(e) = self.conn.send_index(&index).await {
                        warn!(
                            "Failed to send Index for {} to {}: {}",
                            folder_id, self.device_id, e
                        );
                    } else {
                        info!(
                            "Sent Index for {} to {} ({} files)",
                            folder_id, self.device_id, index.files.len()
                        );
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
        loop {
            tokio::select! {
                result = self.conn.recv_message() => {
                    let latency = last_recv.elapsed();
                    match result {
                        Ok((msg_type, payload)) => {
                            metrics::global().record_bep_message_recv(
                                self.device_id.to_string(),
                                &format!("{:?}", msg_type),
                                latency,
                                payload.len() as u64,
                            );
                            last_recv = Instant::now();
                            if let Err(e) = self.handle_message(msg_type, payload).await {
                                warn!("BEP session loop error for {}: {}", self.device_id, e);
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("BEP session loop error for {}: {}", self.device_id, e);
                            break;
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    if let Err(e) = self.conn.send_ping().await {
                        warn!("Failed to send ping to {}: {}", self.device_id, e);
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_message(&self, msg_type: MessageType, payload: bytes::Bytes) -> Result<()> {
        match msg_type {
            MessageType::Ping => {
                self.conn.send_ping().await?;
            }
            MessageType::Index => {
                match bep_protocol::messages::decode_message::<bep_protocol::messages::Index>(&payload)
                {
                    Ok(wire_index) => {
                        let index: syncthing_core::types::Index = wire_index.into();
                        if let Err(e) = self.handler.on_index(self.device_id, index).await {
                            warn!("Failed to handle Index from {}: {}", self.device_id, e);
                        }
                    }
                    Err(e) => warn!("Failed to decode Index from {}: {}", self.device_id, e),
                }
            }
            MessageType::IndexUpdate => {
                match bep_protocol::messages::decode_message::<
                    bep_protocol::messages::IndexUpdate,
                >(&payload)
                {
                    Ok(wire_update) => {
                        let update: syncthing_core::types::IndexUpdate = wire_update.into();
                        if let Err(e) = self.handler.on_index_update(self.device_id, update).await {
                            warn!(
                                "Failed to handle IndexUpdate from {}: {}",
                                self.device_id, e
                            );
                        }
                    }
                    Err(e) => warn!(
                        "Failed to decode IndexUpdate from {}: {}",
                        self.device_id, e
                    ),
                }
            }
            MessageType::Request => {
                match bep_protocol::messages::decode_message::<
                    bep_protocol::messages::Request,
                >(&payload)
                {
                    Ok(req) => {
                        match self.handler.on_block_request(self.device_id, req.clone()).await {
                            Ok(data) => {
                                let resp = bep_protocol::messages::Response {
                                    id: req.id,
                                    data,
                                    code: bep_protocol::messages::ErrorCode::NoError as i32,
                                };
                                match bep_protocol::messages::encode_message(&resp) {
                                    Ok(payload) => {
                                        if let Err(e) = self
                                            .conn
                                            .send_message(MessageType::Response, payload)
                                            .await
                                        {
                                            warn!(
                                                "Failed to send Response to {}: {}",
                                                self.device_id, e
                                            );
                                        }
                                    }
                                    Err(e) => warn!(
                                        "Failed to encode Response for {}: {}",
                                        self.device_id, e
                                    ),
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
                                    let _ = self
                                        .conn
                                        .send_message(MessageType::Response, payload)
                                        .await;
                                }
                            }
                        }
                    }
                    Err(e) => warn!("Failed to decode Request from {}: {}", self.device_id, e),
                }
            }
            MessageType::Response => {
                match bep_protocol::messages::decode_message::<
                    bep_protocol::messages::Response,
                >(&payload)
                {
                    Ok(resp) => {
                        if let Some((_, tx)) = self.pending_responses.remove(&resp.id) {
                            let _ = tx.send(resp);
                        } else {
                            warn!(
                                "Received unmatched Response id={} from {}",
                                resp.id, self.device_id
                            );
                        }
                    }
                    Err(e) => warn!("Failed to decode Response from {}: {}", self.device_id, e),
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
        cluster_config: tokio::sync::Mutex<Option<bep_protocol::messages::ClusterConfig>>,
        index_calls: tokio::sync::Mutex<Vec<(String, DeviceId)>>,
        ping_replies: tokio::sync::Mutex<usize>,
    }

    impl MockHandler {
        fn new() -> Self {
            Self {
                cluster_config: tokio::sync::Mutex::new(None),
                index_calls: tokio::sync::Mutex::new(Vec::new()),
                ping_replies: tokio::sync::Mutex::new(0),
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
                    label: vec![],
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

        let session = BepSession::new(device_id, Arc::clone(&conn_a), handler, pending);
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

        let session = BepSession::new(device_id, Arc::clone(&conn_a), handler, Arc::clone(&pending));
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
}
