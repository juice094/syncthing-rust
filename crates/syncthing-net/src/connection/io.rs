//! BEP connection I/O loops
//!
//! Extracted from connection/mod.rs to keep the public API surface concise.
//! Houses spawn_read_task, spawn_write_task, and spawn_heartbeat_task.

use std::sync::Arc;

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::connection::{BepConnection, ConnectionEvent, Message};
use crate::protocol::MessageHeader;
use syncthing_core::{Result, SyncthingError};

use super::{DEFAULT_MESSAGE_TIMEOUT, MAX_BEP_HEADER_SIZE, MAX_BEP_MESSAGE_SIZE};

impl BepConnection {
    /// 启动读取任务
    pub(crate) fn spawn_read_task(&self) -> tokio::task::JoinHandle<Result<()>> {
        let read_half = Arc::clone(&self.read_half);
        let event_tx = self.event_tx.clone();
        let incoming_tx = self.incoming_tx.clone();
        let inner = Arc::clone(&self.inner);

        tokio::spawn(async move {
            let mut read_half = read_half
                .lock()
                .await
                .take()
                .expect("INVARIANT: start_reader called only once per connection");
            loop {
                // 读取 2 字节 header length
                let mut hdr_len_buf = [0u8; 2];
                match timeout(
                    DEFAULT_MESSAGE_TIMEOUT,
                    read_half.read_exact(&mut hdr_len_buf),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => {
                        continue;
                    }
                }
                let hdr_len = u16::from_be_bytes(hdr_len_buf) as usize;
                if hdr_len > MAX_BEP_HEADER_SIZE {
                    return Err(SyncthingError::protocol(format!(
                        "BEP header too large: {} > {}",
                        hdr_len, MAX_BEP_HEADER_SIZE
                    )));
                }

                // 读取 header 字节
                let mut hdr_buf = vec![0u8; hdr_len];
                match timeout(DEFAULT_MESSAGE_TIMEOUT, read_half.read_exact(&mut hdr_buf)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => return Err(SyncthingError::timeout("header read timeout")),
                }

                // 读取 4 字节 message length
                let mut msg_len_buf = [0u8; 4];
                match timeout(
                    DEFAULT_MESSAGE_TIMEOUT,
                    read_half.read_exact(&mut msg_len_buf),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => return Err(SyncthingError::timeout("message length read timeout")),
                }
                let msg_len = u32::from_be_bytes(msg_len_buf) as usize;
                if msg_len > MAX_BEP_MESSAGE_SIZE {
                    return Err(SyncthingError::protocol(format!(
                        "BEP message too large: {} > {}",
                        msg_len, MAX_BEP_MESSAGE_SIZE
                    )));
                }

                // 读取 message 字节
                let mut msg_buf = vec![0u8; msg_len];
                match timeout(DEFAULT_MESSAGE_TIMEOUT, read_half.read_exact(&mut msg_buf)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(SyncthingError::Io(e)),
                    Err(_) => return Err(SyncthingError::timeout("message read timeout")),
                }

                let bytes_received = 2 + hdr_len + 4 + msg_len;
                let mut stats = inner.stats.write();
                stats.bytes_received += bytes_received as u64;
                stats.last_activity = Some(chrono::Utc::now());
                stats.messages_received += 1;
                drop(stats);

                // 解码 BEP Header
                let bep_header = match <bep_protocol::messages::Header as prost::Message>::decode(
                    &hdr_buf[..],
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        return Err(SyncthingError::protocol(format!(
                            "decode header failed: {}",
                            e
                        )))
                    }
                };

                let Some(header) = MessageHeader::from_bep_header(&bep_header) else {
                    return Err(SyncthingError::protocol(format!(
                        "unknown message type: {}",
                        bep_header.r#type
                    )));
                };

                // 处理 LZ4 压缩
                let msg_buf = if header.compressed {
                    if msg_buf.len() < 4 {
                        return Err(SyncthingError::protocol(
                            "compressed message too short".to_string(),
                        ));
                    }
                    let uncompressed_size =
                        u32::from_be_bytes([msg_buf[0], msg_buf[1], msg_buf[2], msg_buf[3]])
                            as usize;
                    match lz4::block::decompress(&msg_buf[4..], Some(uncompressed_size as i32)) {
                        Ok(decompressed) => decompressed,
                        Err(e) => {
                            return Err(SyncthingError::protocol(format!(
                                "lz4 decompress failed: {}",
                                e
                            )))
                        }
                    }
                } else {
                    msg_buf
                };

                debug!("Received message: {:?}", header.message_type);

                if let Some(device_id) = *inner.device_id.read() {
                    if let Err(e) = event_tx.try_send(ConnectionEvent::MessageReceived {
                        device_id,
                        msg_type: header.message_type,
                    }) {
                        warn!("Failed to send message received event: {}", e);
                    }
                }

                if let Err(e) = incoming_tx.try_send((header.message_type, Bytes::from(msg_buf))) {
                    warn!("Failed to enqueue incoming message: {}", e);
                }
            }
        })
    }

    /// 启动写入任务
    pub(crate) fn spawn_write_task(
        &self,
        message_rx: Arc<Mutex<mpsc::Receiver<Message>>>,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let write_half = Arc::clone(&self.write_half);
        let inner = Arc::clone(&self.inner);

        tokio::spawn(async move {
            let mut write_half = write_half
                .lock()
                .await
                .take()
                .expect("INVARIANT: start_writer called only once per connection");
            let mut rx = message_rx.lock().await;

            while let Some(msg) = rx.recv().await {
                let bep_header = msg.header.to_bep_header();
                let mut hdr_buf = Vec::new();
                if let Err(e) = <bep_protocol::messages::Header as prost::Message>::encode(
                    &bep_header,
                    &mut hdr_buf,
                ) {
                    return Err(SyncthingError::Serialization(e.to_string()));
                }
                let hdr_len = hdr_buf.len();

                // LZ4 compress payload if header requests it
                let (payload, msg_len) = if msg.header.compressed && !msg.payload.is_empty() {
                    let uncompressed = msg.payload.as_ref();
                    let compressed =
                        lz4::block::compress(uncompressed, None, false).map_err(|e| {
                            SyncthingError::Serialization(format!("lz4 compress: {}", e))
                        })?;
                    let uncompressed_len = uncompressed.len() as u32;
                    let mut payload = Vec::with_capacity(4 + compressed.len());
                    payload.extend_from_slice(&uncompressed_len.to_be_bytes());
                    payload.extend_from_slice(&compressed);
                    let payload_len = payload.len();
                    (payload, payload_len)
                } else {
                    (msg.payload.to_vec(), msg.payload.len())
                };

                // header length (2 bytes)
                if let Err(e) = write_half.write_all(&(hdr_len as u16).to_be_bytes()).await {
                    return Err(SyncthingError::Io(e));
                }
                // header
                if let Err(e) = write_half.write_all(&hdr_buf).await {
                    return Err(SyncthingError::Io(e));
                }
                // message length (4 bytes)
                if let Err(e) = write_half.write_all(&(msg_len as u32).to_be_bytes()).await {
                    return Err(SyncthingError::Io(e));
                }
                // payload
                if !payload.is_empty() {
                    if let Err(e) = write_half.write_all(&payload).await {
                        return Err(SyncthingError::Io(e));
                    }
                }
                if let Err(e) = write_half.flush().await {
                    return Err(SyncthingError::Io(e));
                }

                let mut stats = inner.stats.write();
                stats.bytes_sent += (2 + hdr_len + 4 + msg_len) as u64;
                stats.messages_sent += 1;
                stats.last_activity = Some(chrono::Utc::now());
            }

            Ok(())
        })
    }

    /// 启动心跳任务
    pub(crate) fn spawn_heartbeat_task(&self) -> tokio::task::JoinHandle<()> {
        let inner = Arc::clone(&self.inner);
        let heartbeat_interval = inner.heartbeat_interval;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(heartbeat_interval);

            loop {
                interval.tick().await;

                let last_pong = *inner.last_pong.read();
                if last_pong.elapsed() > heartbeat_interval * 3 {
                    // 心跳超时，应该断开连接
                    warn!(
                        "Heartbeat timeout (interval={:?}), closing connection",
                        heartbeat_interval
                    );
                    break;
                }
            }
        })
    }
}
