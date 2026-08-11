//! BEP Session state machine
//!
//! Extracted from session/mod.rs to keep the public API surface concise.
//! Houses the run() lifecycle and handle_message() dispatch logic.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// BEP 单条消息处理超时。防止消息处理中的同步 I/O 阻塞整个会话心跳。
const MESSAGE_HANDLING_TIMEOUT: Duration = Duration::from_secs(30);

use tracing::{debug, info, trace, warn};

use syncthing_core::{ConnectionState, Result, SyncthingError};

use crate::metrics;
use crate::protocol::MessageType;
use crate::session::{BepSession, BepSessionEvent};

impl BepSession {
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
                                    let remote_shared: Vec<String> =
                                        remote_cc.folders.into_iter().map(|f| f.id).collect();
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
                            // BEP 的 Ping 是单向 keepalive，没有 Pong；回复会导致
                            // 双方互答形成 ping 风暴。静默接收即可。
                            trace!("Ping received from {} (no reply per BEP)", self.device_id);
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
                    return Err(SyncthingError::timeout("remote ClusterConfig not received"));
                }
            }
        }

        // 3. Send Index for each folder shared by BOTH sides
        let my_folder_ids: Vec<String> = cc.folders.into_iter().map(|f| f.id).collect();
        let shared_folder_ids: Vec<String> = match &self.remote_shared_folders {
            Some(remote) => my_folder_ids
                .into_iter()
                .filter(|id| remote.contains(id))
                .collect(),
            None => my_folder_ids,
        };
        for folder_id in &shared_folder_ids {
            match self.handler.generate_index(folder_id, self.device_id).await {
                Ok(index) => {
                    let file_count = index.files.len();
                    let last_sequence = index.files.iter().map(|f| f.sequence).max().unwrap_or(0);
                    if let Err(e) = self.conn.send_index(&index).await {
                        warn!(
                            "Failed to send Index for {} to {}: {}",
                            folder_id, self.device_id, e
                        );
                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                    } else {
                        info!(
                            "Sent Index for {} to {} ({} files, last_sequence {})",
                            folder_id, self.device_id, file_count, last_sequence
                        );
                        self.emit(BepSessionEvent::IndexSent {
                            device_id: self.device_id,
                            folder: folder_id.clone(),
                            file_count,
                            last_sequence,
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
        let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
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
                            match tokio::time::timeout(
                                MESSAGE_HANDLING_TIMEOUT,
                                self.handle_message(msg_type, payload),
                            )
                            .await
                            {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => {
                                    warn!("BEP session loop error for {}: {}", self.device_id, e);
                                    session_end_reason = format!("handle_message error: {}", e);
                                    break;
                                }
                                Err(_) => {
                                    warn!(
                                        "BEP message handling timeout for {} after {:?}",
                                        self.device_id, MESSAGE_HANDLING_TIMEOUT
                                    );
                                    session_end_reason = format!(
                                        "handle_message timeout after {:?}",
                                        MESSAGE_HANDLING_TIMEOUT
                                    );
                                    break;
                                }
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
                    if idle > Duration::from_secs(120) {
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

        // 清理未完成的 pending responses，避免内存泄漏
        let pending_count = self.pending_responses.len();
        if pending_count > 0 {
            warn!(
                "Cleaning up {} pending response(s) for {}",
                pending_count, self.device_id
            );
            while let Some(entry) = self.pending_responses.iter().next() {
                let id = *entry.key();
                drop(entry);
                if let Some((_, tx)) = self.pending_responses.remove(&id) {
                    let resp = bep_protocol::messages::Response {
                        id,
                        data: Vec::new(),
                        code: bep_protocol::messages::ErrorCode::Generic as i32,
                    };
                    let _ = tx.send(resp);
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
                // BEP 的 Ping 是单向 keepalive，没有 Pong；回复会导致双方互答
                // 形成 ping 风暴（实测每秒数千条）。收到即证明对端存活，
                // last_recv 已在接收循环更新，无需任何回复。
                trace!("Ping received from {} (no reply per BEP)", self.device_id);
            }
            MessageType::Index => {
                match bep_protocol::messages::decode_message::<bep_protocol::messages::Index>(
                    &payload,
                ) {
                    Ok(wire_index) => {
                        self.metrics.index_received.fetch_add(1, Ordering::Relaxed);
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
                match bep_protocol::messages::decode_message::<bep_protocol::messages::IndexUpdate>(
                    &payload,
                ) {
                    Ok(wire_update) => {
                        self.metrics
                            .index_update_received
                            .fetch_add(1, Ordering::Relaxed);
                        let file_count = wire_update.files.len();
                        let folder = wire_update.folder.clone();
                        let first_seq = wire_update
                            .files
                            .iter()
                            .map(|f| f.sequence)
                            .min()
                            .unwrap_or(0);
                        let last_seq = wire_update
                            .files
                            .iter()
                            .map(|f| f.sequence)
                            .max()
                            .unwrap_or(0);
                        info!(
                            "Received IndexUpdate from {} folder={} files={} seq={}-{}",
                            self.device_id, folder, file_count, first_seq, last_seq
                        );
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
                match bep_protocol::messages::decode_message::<bep_protocol::messages::Request>(
                    &payload,
                ) {
                    Ok(req) => {
                        self.metrics
                            .requests_received
                            .fetch_add(1, Ordering::Relaxed);
                        self.metrics
                            .blocks_requested
                            .fetch_add(1, Ordering::Relaxed);
                        self.emit(BepSessionEvent::BlockRequested {
                            device_id: self.device_id,
                            folder: req.folder.clone(),
                            name: req.name.clone(),
                            offset: req.offset,
                            size: req.size,
                        });
                        match self
                            .handler
                            .on_block_request(self.device_id, req.clone())
                            .await
                        {
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
                                            self.metrics
                                                .messages_sent
                                                .fetch_add(1, Ordering::Relaxed);
                                            self.metrics
                                                .bytes_sent
                                                .fetch_add(payload_len, Ordering::Relaxed);
                                            self.metrics
                                                .blocks_served
                                                .fetch_add(1, Ordering::Relaxed);
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
                                if let Ok(payload) = bep_protocol::messages::encode_message(&resp) {
                                    if let Err(e) =
                                        self.conn.send_message(MessageType::Response, payload).await
                                    {
                                        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                        warn!(
                                            "Failed to send error Response to {}: {}",
                                            self.device_id, e
                                        );
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
                match bep_protocol::messages::decode_message::<bep_protocol::messages::Response>(
                    &payload,
                ) {
                    Ok(resp) => {
                        self.metrics
                            .responses_received
                            .fetch_add(1, Ordering::Relaxed);
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
                self.metrics.closes_received.fetch_add(1, Ordering::Relaxed);
                let reason = String::from_utf8_lossy(&payload);
                info!(
                    "Received Close from {}: {}",
                    self.device_id,
                    if reason.is_empty() {
                        "<no reason>"
                    } else {
                        &reason
                    }
                );
                return Err(SyncthingError::ConnectionClosed);
            }
            _ => {}
        }
        Ok(())
    }
}
