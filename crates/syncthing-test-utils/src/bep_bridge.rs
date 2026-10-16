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

/// 会话就绪信号：`device_id -> 该会话最近一次 ClusterConfig 通告的共享 folder 列表`。
///
/// 由 `install_bep_bridge` 的事件泵在收到 `BepSessionEvent::ClusterConfigComplete` 时写入
/// （生产侧同源实现见 `cmd/syncthing/src/tui/session_logger.rs`）。供测试用
/// `TestNode::wait_for_cluster_config()` / `wait_for_folder_ready()` 等待握手就绪，
/// 替代「先等连接、再靠超时重拨」的竞态规避（见 `docs/KNOWN_ISSUES.md` §23）。
pub type SessionReadiness = Arc<DashMap<DeviceId, Vec<String>>>;

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
        // 会话 churn（§1 ClusterConfig 竞态）会把请求发到一个已无会话消费的连接上，
        // 表现为长时间 "response timeout"；实测同一块在数秒后重试即可成功。
        // 因此每次重试都重新取连接，而不是一次用到死。
        //
        // 单次超时取 5s 而非 10s：测试用的块都很小（本机 loopback 下毫秒级完成），
        // 10s 只是把「落在失效连接上」的等待成本放大到 3×10s，进而吃掉 `e2e_crud`
        // 断言 60s 预算的一半以上（见 docs/KNOWN_ISSUES.md §23）。缩短后重试能更快
        // 换到获胜连接，同时保留 3 次重试的容错。
        const ATTEMPTS: usize = 3;
        const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

        let devices = self.manager.connected_devices();
        if devices.is_empty() {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                "No connected devices".to_string(),
            ));
        }
        let device_id = devices[0]; // simple: just use first peer for test harness

        let mut last_error = String::from("no attempt made");
        for attempt in 0..ATTEMPTS {
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

            let conn = match self.manager.get_connection(&device_id) {
                Some(conn) => conn,
                None => {
                    last_error = format!("no connection to {}", device_id);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };

            let (tx, rx) = oneshot::channel();
            self.pending_responses.insert(id, tx);

            if let Err(e) = conn.send_message(MessageType::Request, payload).await {
                self.pending_responses.remove(&id);
                last_error = format!("send: {}", e);
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }

            let response = match tokio::time::timeout(ATTEMPT_TIMEOUT, rx).await {
                Ok(Ok(response)) => response,
                Ok(Err(_)) => {
                    self.pending_responses.remove(&id);
                    last_error = "response channel closed".to_string();
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                Err(_) => {
                    self.pending_responses.remove(&id);
                    last_error = "response timeout".to_string();
                    debug!(
                        "[test-harness] block request for {} (block {}) timed out on attempt {}, retrying",
                        file, block_no, attempt
                    );
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            };

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
            return Ok(Bytes::from(response.data));
        }

        Err(syncthing_sync::SyncError::pull(
            file.to_string(),
            format!("{} (after {} attempts)", last_error, ATTEMPTS),
        ))
    }
}

/// 发送一条 IndexUpdate，连接未就绪或发送失败时按 500ms 退避重试（约 30s）。
///
/// BEP 会话就绪与 `LocalIndexUpdated` 事件之间存在竞态：旧实现遇到「无连接」
/// 只打一行 warn 就丢弃，导致索引更新永久丢失、测试表现为「连接正常但文件
/// 永不同步」（见 `docs/KNOWN_ISSUES.md` §23）。
fn send_index_update_with_retry(
    handle: ConnectionManagerHandle,
    device_id: DeviceId,
    update: syncthing_core::IndexUpdate,
) {
    const RETRY_INTERVAL: Duration = Duration::from_millis(500);
    const MAX_ATTEMPTS: u32 = 60;

    tokio::spawn(async move {
        for attempt in 0..MAX_ATTEMPTS {
            if let Some(conn) = handle.get_connection(&device_id) {
                match conn.send_index_update(&update).await {
                    Ok(()) => {
                        if attempt == 0 {
                            info!(
                                "[test-harness] Sent IndexUpdate to {} for folder {} ({} files)",
                                device_id,
                                update.folder,
                                update.files.len()
                            );
                        } else {
                            info!(
                                "[test-harness] IndexUpdate to {} for folder {} delivered after {} retries",
                                device_id, update.folder, attempt
                            );
                        }
                        return;
                    }
                    Err(e) => debug!(
                        "[test-harness] IndexUpdate to {} failed (attempt {}): {}",
                        device_id, attempt, e
                    ),
                }
            }
            tokio::time::sleep(RETRY_INTERVAL).await;
        }
        warn!(
            "[test-harness] IndexUpdate to {} for folder {} undelivered after {:?}",
            device_id,
            update.folder,
            RETRY_INTERVAL * MAX_ATTEMPTS
        );
    });
}

/// Install the BEP session bridge on a `ConnectionManager`.
///
/// Returns `(PendingResponses, SessionReadiness)`: the block-request correlation
/// map (so callers can wire a matching `TestBlockSource` into the `SyncService`)
/// and the ClusterConfig readiness signal (`TestNode::wait_for_cluster_config`).
///
/// Must be called **before** `manager.start()` so the callbacks are
/// registered before any connection arrives.
pub fn install_bep_bridge(
    manager: &ConnectionManager,
    sync_service: &Arc<SyncService>,
    handle: ConnectionManagerHandle,
) -> (PendingResponses, SessionReadiness) {
    let pending_responses: PendingResponses = Arc::new(DashMap::new());
    let readiness: SessionReadiness = Arc::new(DashMap::new());
    let session_handles: Arc<DashMap<DeviceId, JoinHandle<()>>> = Arc::new(DashMap::new());

    // on_connected: spawn BepSession::run()
    let sync_service_c = Arc::clone(sync_service);
    let handle_c = handle.clone();
    let pending_c = Arc::clone(&pending_responses);
    let readiness_c = Arc::clone(&readiness);
    let sessions_c = Arc::clone(&session_handles);
    manager.on_connected(move |device_id| {
        info!("[test-harness] Device connected: {}", device_id);
        let sync_service = Arc::clone(&sync_service_c);
        let handle = handle_c.clone();
        let pending = Arc::clone(&pending_c);
        let readiness = Arc::clone(&readiness_c);
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
                // 事件泵：记录 ClusterConfig 就绪信号，供等待原语使用。
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        if let BepSessionEvent::ClusterConfigComplete {
                            device_id,
                            shared_folders,
                        } = event
                        {
                            readiness.insert(device_id, shared_folders);
                        }
                    }
                });
                let handler = TestBepHandler {
                    sync_service: Arc::clone(&sync_service),
                };
                if let Some(conn) = handle.get_connection(&device_id) {
                    // 只断开「本会话对应的这一条连接」。按 device_id 断开会在
                    // race resolution 换连接时连带杀掉对端刚建立的获胜连接，
                    // 双方随之反复重拨，块请求找不到服务方而超时（§23 实测链路）。
                    let conn_id = conn.id();
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
                    let _ = handle
                        .disconnect_connection(&conn_id, "test session ended")
                        .await;
                } else {
                    warn!("[test-harness] No connection for {}", device_id);
                }
            });
            sessions.insert(device_id, handle2);
        });
    });

    // on_disconnected
    let sync_service_d = Arc::clone(sync_service);
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
            // 不 abort：race resolution 替换连接时，on_disconnected 可能在 on_connected
            // 启动新 BepSession 之后才执行，abort 会误杀新 session。旧 session 自己会在
            // 连接关闭后检测到 recv/send 错误并退出。
            let _ = sessions.remove(&device_id);
        });
    });

    // Spawn background task: forward LocalIndexUpdated events -> BEP IndexUpdate
    // messages to all connected peers. Without this, scan_folder() changes never
    // leave the local node.
    let sync_service_e = Arc::clone(sync_service);
    let handle_e = handle;
    tokio::spawn(async move {
        let mut subscriber = sync_service_e.events().subscribe();
        info!("[test-harness] IndexUpdate forwarding task started");
        loop {
            match subscriber.recv().await {
                Some(syncthing_sync::SyncEvent::LocalIndexUpdated { folder, mut files }) => {
                    info!(
                        "[test-harness] LocalIndexUpdated event: folder={} files={}",
                        folder,
                        files.len()
                    );
                    // BEP convention: deleted files must have empty blocks
                    for file in &mut files {
                        if file.is_deleted() {
                            file.blocks.clear();
                        }
                    }
                    let update = syncthing_core::types::IndexUpdate { folder, files };
                    // 按「配置中共享该 folder 的设备」派发，而非当前已连接设备：
                    // 会话就绪与事件之间存在竞态，连接暂时缺失时若直接跳过，
                    // 该索引更新会被静默丢弃且永不重发（§23 根因）。
                    let config = sync_service_e.get_config().await.unwrap_or_default();
                    let targets: Vec<DeviceId> = config
                        .folders
                        .iter()
                        .find(|f| f.id == update.folder)
                        .map(|f| f.devices.clone())
                        .unwrap_or_default();
                    if targets.is_empty() {
                        warn!(
                            "[test-harness] no configured peer shares folder {}, index update dropped",
                            update.folder
                        );
                        continue;
                    }
                    for device_id in targets {
                        send_index_update_with_retry(handle_e.clone(), device_id, update.clone());
                    }
                }
                Some(_) => {}
                None => break,
            }
        }
    });

    (pending_responses, readiness)
}
