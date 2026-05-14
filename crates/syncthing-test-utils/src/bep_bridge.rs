//! T2.5 — BEP session bridging for TestNode harness.
//!
//! Production `start_daemon` (cmd/syncthing/src/tui/daemon_runner.rs) wires:
//! 1. `ConnectionManager::on_connected` → spawns `BepSession::run()` per peer
//! 2. `ConnectionManager::on_disconnected` → aborts the session
//! 3. `SyncService::set_block_source` → routes pullers block requests via the manager
//! 4. A shared `pending_responses` map for `Request`/`Response` correlation
//!
//! TestNode by itself only sets up the connection layer (TLS + Hello). Without
//! this module, the BEP session is never started — no ClusterConfig, no Index,
//! no end-to-end sync.
//!
//! This module ports the minimum subset of the production pipeline so harness
//! tests can verify cluster config exchange, index propagation, and block
//! transfer.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use syncthing_core::{DeviceId, DeviceIdentity, SyncthingError};
use syncthing_net::{
    protocol::MessageType, BepSession, BepSessionEvent, BepSessionHandler, ConnectionManager,
    ConnectionManagerHandle,
};
use syncthing_sync::{puller::BlockSource, SyncManager, SyncService};

/// Shared map of in-flight BEP block-request correlations.
///
/// Keyed by `Request.id` (i32). On receipt of a `Response`, BepSession`s
/// run loop pops the matching sender and delivers the payload.
pub type PendingResponses = Arc<DashMap<i32, oneshot::Sender<bep_protocol::messages::Response>>>;

/// BEP session handler suitable for test harness.
///
/// Mirrors `cmd/syncthing/src/tui/bep_handler.rs::DaemonBepHandler`; ported
/// here to avoid pulling the cmd/syncthing crate into test-utils.
pub struct TestBepHandler {
    pub sync_service: Arc<SyncService>,
}

#[async_trait]
impl BepSessionHandler for TestBepHandler {
    async fn generate_cluster_config(
        &self,
        device_id: DeviceId,
    ) -> syncthing_core::Result<bep_protocol::messages::ClusterConfig> {
        let config = self.sync_service.get_config().await.unwrap_or_default();
        let local_id = config.local_device_id.unwrap_or_default();
        let folders: Vec<bep_protocol::messages::WireFolder> = config
            .folders
            .iter()
            .filter(|f| f.devices.contains(&device_id))
            .map(|f| {
                let mut devices: Vec<bep_protocol::messages::WireDevice> = f
                    .devices
                    .iter()
                    .map(|d| bep_protocol::messages::WireDevice {
                        id: d.as_bytes().to_vec(),
                        name: String::new(),
                        addresses: vec![],
                        compression: bep_protocol::messages::Compression::Metadata as i32,
                        cert_name: String::new(),
                        max_sequence: 0,
                        introducer: false,
                        index_id: 0,
                        skip_introduction_removals: false,
                        encryption_password_token: Vec::new(),
                    })
                    .collect();
                if !f.devices.contains(&local_id) {
                    devices.push(bep_protocol::messages::WireDevice {
                        id: local_id.as_bytes().to_vec(),
                        name: String::new(),
                        addresses: vec![],
                        compression: bep_protocol::messages::Compression::Metadata as i32,
                        cert_name: String::new(),
                        max_sequence: 0,
                        introducer: false,
                        index_id: 0,
                        skip_introduction_removals: false,
                        encryption_password_token: Vec::new(),
                    });
                }
                bep_protocol::messages::WireFolder {
                    id: f.id.clone(),
                    label: f.label.clone().unwrap_or_default(),
                    r#type: bep_protocol::messages::FolderType::SendReceive as i32,
                    stop_reason: bep_protocol::messages::FolderStopReason::Running as i32,
                    devices,
                }
            })
            .collect();
        Ok(bep_protocol::messages::ClusterConfig {
            folders,
            secondary: false,
        })
    }

    async fn generate_index(
        &self,
        folder_id: &str,
        _device_id: DeviceId,
    ) -> syncthing_core::Result<syncthing_core::types::Index> {
        let mut files = self
            .sync_service
            .generate_index_update(folder_id, 0)
            .await
            .map_err(|e| {
                SyncthingError::internal(format!("generate_index_update failed: {}", e))
            })?;
        for file in &mut files {
            if file.is_deleted() {
                file.blocks.clear();
            }
        }
        Ok(syncthing_core::types::Index {
            folder: folder_id.to_string(),
            files,
        })
    }

    async fn on_index(
        &self,
        device_id: DeviceId,
        index: syncthing_core::types::Index,
    ) -> syncthing_core::Result<()> {
        let folder = index.folder.clone();
        self.sync_service
            .handle_index(&folder, device_id, index)
            .await
            .map_err(|e| SyncthingError::internal(format!("handle_index failed: {:?}", e)))?;
        Ok(())
    }

    async fn on_index_update(
        &self,
        device_id: DeviceId,
        update: syncthing_core::types::IndexUpdate,
    ) -> syncthing_core::Result<()> {
        let folder = update.folder.clone();
        self.sync_service
            .handle_index_update(&folder, device_id, update)
            .await
            .map_err(|e| {
                SyncthingError::internal(format!("handle_index_update failed: {:?}", e))
            })?;
        Ok(())
    }

    async fn on_block_request(
        &self,
        _device_id: DeviceId,
        req: bep_protocol::messages::Request,
    ) -> std::result::Result<Vec<u8>, bep_protocol::messages::ErrorCode> {
        self.sync_service
            .handle_block_request(&req)
            .await
            .map_err(|e| e.error_code())
    }
}

/// Block source used by SyncService puller to fetch remote blocks over BEP.
///
/// Mirrors `cmd/syncthing/src/main.rs::ManagerBlockSource`.
pub struct TestBlockSource {
    pub manager: ConnectionManagerHandle,
    pub next_id: AtomicI32,
    pub pending_responses: PendingResponses,
}

#[async_trait]
impl BlockSource for TestBlockSource {
    async fn request_block(
        &self,
        folder: &str,
        file: &str,
        block: &syncthing_core::types::BlockInfo,
        block_no: usize,
    ) -> syncthing_sync::Result<Bytes> {
        let devices = self.manager.connected_devices();
        if devices.is_empty() {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                "No connected devices".to_string(),
            ));
        }
        let device_id = devices[0]; // simple: just use first peer for test harness

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = bep_protocol::messages::Request {
            id,
            folder: folder.to_string(),
            name: file.to_string(),
            offset: block.offset,
            size: block.size,
            hash: block.hash.clone(),
            from_temporary: false,
            block_no: block_no as i32,
        };
        let payload = bep_protocol::messages::encode_message(&request).map_err(|e| {
            syncthing_sync::SyncError::pull(file.to_string(), format!("encode failed: {}", e))
        })?;

        let conn = self.manager.get_connection(&device_id).ok_or_else(|| {
            syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("no connection to {}", device_id),
            )
        })?;

        let (tx, rx) = oneshot::channel();
        self.pending_responses.insert(id, tx);

        conn.send_message(MessageType::Request, payload)
            .await
            .map_err(|e| {
                syncthing_sync::SyncError::pull(file.to_string(), format!("send: {}", e))
            })?;

        let response = tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| {
                syncthing_sync::SyncError::pull(file.to_string(), "response timeout".to_string())
            })?
            .map_err(|_| {
                syncthing_sync::SyncError::pull(
                    file.to_string(),
                    "response channel closed".to_string(),
                )
            })?;

        debug!(
            "Block response from {}: code={} data_len={}",
            device_id,
            response.code,
            response.data.len()
        );

        if response.code != bep_protocol::messages::ErrorCode::NoError as i32 {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("remote error code {} from {}", response.code, device_id),
            ));
        }
        if response.data.len() != block.size as usize {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                format!(
                    "block size mismatch: expected {} got {}",
                    block.size,
                    response.data.len()
                ),
            ));
        }
        Ok(Bytes::from(response.data))
    }
}

/// Install the BEP session bridge on a `ConnectionManager`.
///
/// Returns the `PendingResponses` map so callers can wire a matching
/// `TestBlockSource` into the `SyncService`.
///
/// Must be called **before** `manager.start()` so the callbacks are
/// registered before any connection arrives.
pub fn install_bep_bridge(
    manager: &ConnectionManager,
    sync_service: Arc<SyncService>,
    handle: ConnectionManagerHandle,
) -> PendingResponses {
    let pending_responses: PendingResponses = Arc::new(DashMap::new());
    let session_handles: Arc<DashMap<DeviceId, JoinHandle<()>>> = Arc::new(DashMap::new());

    // on_connected: spawn BepSession::run()
    let sync_service_c = Arc::clone(&sync_service);
    let handle_c = handle.clone();
    let pending_c = Arc::clone(&pending_responses);
    let sessions_c = Arc::clone(&session_handles);
    manager.on_connected(move |device_id| {
        info!("[test-harness] Device connected: {}", device_id);
        let sync_service = Arc::clone(&sync_service_c);
        let handle = handle_c.clone();
        let pending = Arc::clone(&pending_c);
        let sessions = Arc::clone(&sessions_c);
        tokio::spawn(async move {
            if let Err(e) = sync_service.connect_device(device_id).await {
                warn!("[test-harness] connect_device failed: {}", e);
            }
            if let Some((_, old)) = sessions.remove(&device_id) {
                old.abort();
            }
            let handle2 = tokio::spawn(async move {
                let (event_tx, mut event_rx) = mpsc::channel::<BepSessionEvent>(256);
                tokio::spawn(async move {
                    while event_rx.recv().await.is_some() {}
                });
                let handler = TestBepHandler {
                    sync_service: Arc::clone(&sync_service),
                };
                if let Some(conn) = handle.get_connection(&device_id) {
                    let session = BepSession::with_events(
                        Arc::new(DeviceIdentity::new(device_id)),
                        conn,
                        Arc::new(handler),
                        pending,
                        event_tx,
                    );
                    if let Err(e) = session.run().await {
                        warn!("[test-harness] BepSession for {} ended: {}", device_id, e);
                    }
                } else {
                    warn!("[test-harness] No connection for {}", device_id);
                }
                let _ = handle.disconnect(&device_id, "test session ended").await;
            });
            sessions.insert(device_id, handle2);
        });
    });

    // on_disconnected
    let sync_service_d = Arc::clone(&sync_service);
    let sessions_d = Arc::clone(&session_handles);
    manager.on_disconnected(move |device_id, reason| {
        info!(
            "[test-harness] Device disconnected: {} - {}",
            device_id, reason
        );
        let sync_service = Arc::clone(&sync_service_d);
        let sessions = Arc::clone(&sessions_d);
        tokio::spawn(async move {
            if let Err(e) = sync_service.disconnect_device(device_id).await {
                warn!("[test-harness] disconnect_device failed: {}", e);
            }
            if let Some((_, h)) = sessions.remove(&device_id) {
                h.abort();
            }
        });
    });

    pending_responses
}
