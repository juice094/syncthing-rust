//! Session event logging task
//!
//! 2026-05-13 (T1.3)：从 `daemon_runner.rs` 抽离，专责为单个 BEP session 记录事件日志。

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use syncthing_core::DeviceId;
use syncthing_net::BepSessionEvent;

/// 为某一设备的 BepSession 启动事件日志记录任务。
///
/// 监听 `event_rx`，将每个 `BepSessionEvent` 转为人类可读的 info/warn 日志，
/// 同时在收到 `ClusterConfigComplete` 时更新 `shared_folders_map`。
pub fn spawn_session_event_logger(
    event_device_id: DeviceId,
    mut event_rx: UnboundedReceiver<BepSessionEvent>,
    shared_folders_map: Arc<DashMap<DeviceId, Vec<String>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match &event {
                BepSessionEvent::ClusterConfigComplete { shared_folders, .. } => {
                    info!(
                        "[{}] ClusterConfig complete, shared folders: {:?}",
                        event_device_id, shared_folders
                    );
                    shared_folders_map.insert(event_device_id, shared_folders.clone());
                }
                BepSessionEvent::IndexSent {
                    folder, file_count, ..
                } => {
                    info!(
                        "[{}] Index sent for {} ({} files)",
                        event_device_id, folder, file_count
                    );
                }
                BepSessionEvent::IndexReceived {
                    folder, file_count, ..
                } => {
                    info!(
                        "[{}] Index received for {} ({} files)",
                        event_device_id, folder, file_count
                    );
                }
                BepSessionEvent::IndexUpdateReceived {
                    folder, file_count, ..
                } => {
                    info!(
                        "[{}] IndexUpdate received for {} ({} files)",
                        event_device_id, folder, file_count
                    );
                }
                BepSessionEvent::BlockRequested {
                    folder,
                    name,
                    offset,
                    size,
                    ..
                } => {
                    info!(
                        "[{}] Block requested: {}/{} offset={} size={}",
                        event_device_id, folder, name, offset, size
                    );
                }
                BepSessionEvent::HeartbeatTimeout { last_recv_age, .. } => {
                    warn!(
                        "[{}] Heartbeat timeout (idle {:?})",
                        event_device_id, last_recv_age
                    );
                }
                BepSessionEvent::PeerSyncState { folder, .. } => {
                    info!(
                        "[{}] Peer sync state changed for {}",
                        event_device_id, folder
                    );
                }
                BepSessionEvent::SessionEnded { reason, .. } => {
                    info!("[{}] Session ended: {}", event_device_id, reason);
                }
            }
        }
    })
}
