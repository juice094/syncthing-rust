//! Local index update propagation
//!
//! 2026-05-13 (T1.3)：从 `daemon_runner.rs` 抽离。监听本地 `LocalIndexUpdated`
//! 事件，向共享该文件夹的已连接对端发送 BEP `IndexUpdate` 消息。

use std::sync::Arc;

use dashmap::DashMap;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use syncthing_core::DeviceId;
use syncthing_net::protocol::MessageType;
use syncthing_net::ConnectionManagerHandle;
use syncthing_sync::{events::SyncEvent, SyncService};

/// 启动本地索引更新传播任务。
///
/// 订阅 `sync_service.events()`，当收到 `LocalIndexUpdated` 时：
/// 1. 清空被删除文件的 block list（BEP 协议要求）
/// 2. 编码为 `bep_protocol::messages::IndexUpdate`
/// 3. 向所有已连接、且根据 `device_shared_folders` 共享该文件夹的对端发送
pub fn spawn_index_propagation_loop(
    sync_service: Arc<SyncService>,
    handle: ConnectionManagerHandle,
    device_shared_folders: Arc<DashMap<DeviceId, Vec<String>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut subscriber = sync_service.events().subscribe();
        while let Some(event) = subscriber.recv().await {
            if let SyncEvent::LocalIndexUpdated { folder, files } = event {
                if files.is_empty() {
                    continue;
                }
                // 防御性清空：确保 deleted 文件的 block list 为空（BEP 协议要求）
                let mut safe_files = files.clone();
                for file in &mut safe_files {
                    if file.is_deleted() {
                        file.blocks.clear();
                    }
                }
                let update = syncthing_core::types::IndexUpdate {
                    folder: folder.clone(),
                    files: safe_files,
                };
                let wire_update: bep_protocol::messages::IndexUpdate = update.into();
                match bep_protocol::messages::encode_message(&wire_update) {
                    Ok(payload) => {
                        for device_id in handle.connected_devices() {
                            // 只发送给共享该文件夹的设备
                            let should_send = match device_shared_folders.get(&device_id) {
                                Some(entry) => entry.value().contains(&folder),
                                None => {
                                    // 尚未收到 ClusterConfig，保守起见不发送
                                    false
                                }
                            };
                            if !should_send {
                                continue;
                            }
                            if let Some(conn) = handle.get_connection(&device_id) {
                                if let Err(e) = conn
                                    .send_message(MessageType::IndexUpdate, payload.clone())
                                    .await
                                {
                                    warn!(
                                        "Failed to send IndexUpdate to {} for {}: {}",
                                        device_id, folder, e
                                    );
                                } else {
                                    info!(
                                        "Sent IndexUpdate for {} to {} ({} files)",
                                        folder,
                                        device_id,
                                        files.len()
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to encode IndexUpdate for {}: {}", folder, e);
                    }
                }
            }
        }
    })
}
