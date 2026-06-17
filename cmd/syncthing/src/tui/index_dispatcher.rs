//! Local index update propagation
//!
//! 2026-05-13 (T1.3)：从 `daemon_runner.rs` 抽离。监听本地 `LocalIndexUpdated`
//! 事件，向共享该文件夹的已连接对端发送 BEP `IndexUpdate` 消息。
//!
//! 2026-06-16：修复手机 Syncthing-Fork 断开问题——必须在对端收到初始 `Index`
//! 之后才能发送 `IndexUpdate`，且序列号字段必须正确填充。
//!
//! 2026-06-16 (Phase B)：大变更集自动分片，每片 ≤1MiB，避免单个 IndexUpdate
//! 接近 BEP 消息上限并降低对端解码内存尖峰。

const MAX_INDEX_UPDATE_BYTES: usize = 1_000_000;

use std::sync::Arc;

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use syncthing_core::DeviceId;
use syncthing_net::protocol::MessageType;
use syncthing_net::ConnectionManagerHandle;
use syncthing_sync::{events::SyncEvent, SyncService};

/// 启动本地索引更新传播任务。
///
/// 订阅 `sync_service.events()`，当收到 `LocalIndexUpdated` 时：
/// 1. 清空被删除文件的 block list（BEP 协议要求）
/// 2. 仅保留序列号大于已向该设备发送过的最大序列号的文件（去重/防乱序）
/// 3. 在确认已向该设备发送过初始 `Index` 后，编码并发送 `IndexUpdate`
/// 4. 更新已发送的最大序列号
pub fn spawn_index_propagation_loop(
    sync_service: Arc<SyncService>,
    handle: ConnectionManagerHandle,
    device_shared_folders: Arc<DashMap<DeviceId, Vec<String>>>,
    indexed_folders_map: Arc<DashMap<(DeviceId, String), u64>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut subscriber = sync_service.events().subscribe();
        while let Some(event) = subscriber.recv().await {
            if let SyncEvent::LocalIndexUpdated { folder, files } = event {
                if files.is_empty() {
                    continue;
                }

                for device_id in handle.connected_devices() {
                    // 只发送给共享该文件夹的设备
                    let shares_folder = device_shared_folders
                        .get(&device_id)
                        .map(|entry| entry.value().contains(&folder))
                        .unwrap_or(false);
                    if !shares_folder {
                        continue;
                    }

                    // 必须已经向该设备发送过初始 Index
                    let prev_sequence = match indexed_folders_map.get(&(device_id, folder.clone()))
                    {
                        Some(entry) => *entry.value(),
                        None => {
                            debug!(
                                device = %device_id.short_id(),
                                folder = %folder,
                                "Skipping IndexUpdate: initial Index not yet sent"
                            );
                            continue;
                        }
                    };

                    // 防御性处理：
                    // - deleted 文件的 block list 必须为空（BEP 协议要求）
                    // - 零字节普通文件必须有一个空 block，否则对端会报 "file with empty block list"
                    let mut files_to_send: Vec<syncthing_core::types::FileInfo> = files
                        .iter()
                        .filter(|f| f.sequence > prev_sequence)
                        .cloned()
                        .collect();
                    for file in &mut files_to_send {
                        if file.is_deleted() {
                            file.blocks.clear();
                        } else if file.file_type == syncthing_core::types::FileType::File
                            && file.size == 0
                            && file.blocks.is_empty()
                        {
                            file.blocks.push(syncthing_core::types::BlockInfo {
                                size: 0,
                                hash: Sha256::new().finalize().to_vec(),
                                offset: 0,
                            });
                        }
                    }

                    if files_to_send.is_empty() {
                        continue;
                    }

                    // 按 sequence 排序，保证分片边界连续
                    files_to_send.sort_by_key(|f| f.sequence);

                    // 按编码后大小分片
                    let chunks = build_index_update_chunks(&folder, prev_sequence, &files_to_send);
                    if chunks.is_empty() {
                        continue;
                    }
                    let total_chunks = chunks.len();

                    let mut last_sent_sequence = prev_sequence;
                    let mut send_failed = false;

                    for (chunk_index, (chunk_prev, chunk_files)) in chunks.into_iter().enumerate() {
                        let chunk_last = chunk_files
                            .iter()
                            .map(|f| f.sequence)
                            .max()
                            .unwrap_or(chunk_prev);

                        let wire_update = bep_protocol::messages::IndexUpdate {
                            folder: folder.clone(),
                            files: chunk_files.iter().cloned().map(Into::into).collect(),
                            last_sequence: chunk_last.min(i64::MAX as u64) as i64,
                            prev_sequence: chunk_prev.min(i64::MAX as u64) as i64,
                        };

                        let payload = match bep_protocol::messages::encode_message(&wire_update) {
                            Ok(p) => p,
                            Err(e) => {
                                warn!(
                                    "Failed to encode IndexUpdate chunk {}/{} for {} to {}: {}",
                                    chunk_index + 1,
                                    total_chunks,
                                    folder,
                                    device_id,
                                    e
                                );
                                send_failed = true;
                                break;
                            }
                        };

                        if let Some(conn) = handle.get_connection(&device_id) {
                            let payload_len = payload.len();
                            match conn.send_message(MessageType::IndexUpdate, payload).await {
                                Ok(_) => {
                                    info!(
                                        "Sent IndexUpdate chunk {}/{} for {} to {} ({} files, prev={}, last={})",
                                        chunk_index + 1,
                                        total_chunks,
                                        folder,
                                        device_id,
                                        wire_update.files.len(),
                                        wire_update.prev_sequence,
                                        wire_update.last_sequence
                                    );
                                    indexed_folders_map
                                        .insert((device_id, folder.clone()), chunk_last);
                                    last_sent_sequence = chunk_last;
                                    sync_service.events().publish(
                                        SyncEvent::IndexUpdateChunkSent {
                                            folder: folder.clone(),
                                            device: device_id,
                                            chunk_index,
                                            total_chunks,
                                            files: wire_update.files.len(),
                                            bytes: payload_len,
                                        },
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to send IndexUpdate chunk {}/{} to {} for {}: {}",
                                        chunk_index + 1,
                                        total_chunks,
                                        device_id,
                                        folder,
                                        e
                                    );
                                    send_failed = true;
                                    break;
                                }
                            }
                        } else {
                            send_failed = true;
                            break;
                        }
                    }

                    if send_failed && last_sent_sequence > prev_sequence {
                        debug!(
                            device = %device_id.short_id(),
                            folder = %folder,
                            last_sent = last_sent_sequence,
                            "Partial IndexUpdate sent; remaining files will be retried on next event or reconnect"
                        );
                    }
                }
            }
        }
    })
}

/// 将待发送的索引文件按编码后体积分片。
///
/// 返回的每个元素为 `(chunk_prev_sequence, chunk_files)`，其中 `chunk_prev_sequence`
/// 是该分片应填写的 `prev_sequence`，`chunk_files` 已按 sequence 升序排列。
fn build_index_update_chunks(
    folder: &str,
    initial_prev: u64,
    files: &[syncthing_core::types::FileInfo],
) -> Vec<(u64, Vec<syncthing_core::types::FileInfo>)> {
    let mut chunks: Vec<(u64, Vec<syncthing_core::types::FileInfo>)> = Vec::new();
    let mut current: Vec<syncthing_core::types::FileInfo> = Vec::new();
    let mut chunk_prev = initial_prev;

    for file in files {
        current.push(file.clone());
        let candidate_last = current
            .iter()
            .map(|f| f.sequence)
            .max()
            .unwrap_or(chunk_prev);
        let wire = bep_protocol::messages::IndexUpdate {
            folder: folder.to_string(),
            files: current.iter().cloned().map(Into::into).collect(),
            last_sequence: candidate_last.min(i64::MAX as u64) as i64,
            prev_sequence: chunk_prev.min(i64::MAX as u64) as i64,
        };

        match bep_protocol::messages::encode_message(&wire) {
            Ok(payload) if payload.len() <= MAX_INDEX_UPDATE_BYTES => {
                // 当前分片仍在上限内，继续追加
            }
            _ => {
                // 超出上限：把最后一个文件移出，结束当前分片
                current.pop();
                if !current.is_empty() {
                    let last = current
                        .iter()
                        .map(|f| f.sequence)
                        .max()
                        .unwrap_or(chunk_prev);
                    chunks.push((chunk_prev, std::mem::take(&mut current)));
                    chunk_prev = last;
                }
                current.push(file.clone());
            }
        }
    }

    if !current.is_empty() {
        chunks.push((chunk_prev, current));
    }

    chunks
}
