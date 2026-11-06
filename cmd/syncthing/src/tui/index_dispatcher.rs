//! Local index update propagation.
//!
//! Subscribes to local `LocalIndexUpdated` events and forwards BEP
//! `IndexUpdate` messages to connected peers that share the folder. Two
//! invariants apply (see "docs/design/ADR-20261105-index-update-dispatch.md"):
//!
//! 1. `IndexUpdate` must only be sent after the peer has received the initial
//!    `Index` for that folder, with correct sequence fields.
//! 2. Large change sets are chunked so a single `IndexUpdate` stays well below
//!    the BEP message limit, bounding the peer's decode memory spike.

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

/// Starts the local index update propagation task.
///
/// For each `LocalIndexUpdated` event:
/// 1. clears block lists of deleted files (required by BEP);
/// 2. keeps only files whose sequence exceeds the highest sequence already sent
///    to that device (dedup / out-of-order protection);
/// 3. encodes and sends `IndexUpdate` only after the device's initial `Index`
///    has been confirmed sent;
/// 4. records the new highest sent sequence.
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
                    let shares_folder = device_shared_folders
                        .get(&device_id)
                        .map(|entry| entry.value().contains(&folder))
                        .unwrap_or(false);
                    if !shares_folder {
                        continue;
                    }

                    // The initial Index must have been sent to this device first.
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

                    // BEP requires deleted files to carry an empty block list,
                    // and zero-byte regular files must carry one empty block so
                    // the peer does not reject them as "file with empty block list".
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

                    files_to_send.sort_by_key(|f| f.sequence);

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

/// Splits the pending index files into chunks by encoded size.
///
/// Each element is `(chunk_prev_sequence, chunk_files)`, where
/// `chunk_prev_sequence` is the `prev_sequence` the chunk must carry and
/// `chunk_files` is sorted by ascending sequence.
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
            Ok(payload) if payload.len() <= MAX_INDEX_UPDATE_BYTES => {}
            _ => {
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
