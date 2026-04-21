//! DERP Pipe：将 DERP 消息转发包装为 `ReliablePipe` 字节流。
//!
//! DERP 本质上是基于消息的转发协议（SendPacket / RecvPacket）。
//! 为了适配 BEP 协议（需要可靠字节流），我们将 DERP 帧的 payload
//! 拼接为连续字节流，写入时自动分片为 SendPacket 发送。

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;

use std::task::{Context, Poll};

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use syncthing_core::{PathQuality, ReliablePipe, TransportType};
use syncthing_core::DeviceId;

use super::protocol::{Frame};

/// 内部发送任务：从 channel 接收已编码的帧数据，写入 TCP stream
async fn send_task(mut write_half: OwnedWriteHalf, mut rx: mpsc::UnboundedReceiver<Vec<u8>>) {
    while let Some(data) = rx.recv().await {
        trace!("DERP send_task writing {} bytes", data.len());
        if let Err(e) = tokio::io::AsyncWriteExt::write_all(&mut write_half, &data).await {
            if e.kind() != io::ErrorKind::BrokenPipe {
                debug!("DERP send_task write error: {}", e);
            }
            break;
        }
    }
    let _ = tokio::io::AsyncWriteExt::shutdown(&mut write_half).await;
    debug!("DERP send_task exited");
}

/// 内部接收任务：从 TCP stream 读取帧，将 RecvPacket 的 payload 传给读端
async fn recv_task(mut read_half: OwnedReadHalf, read_tx: mpsc::UnboundedSender<Bytes>) {
    let mut len_buf = [0u8; 4];
    loop {
        match tokio::io::AsyncReadExt::read_exact(&mut read_half, &mut len_buf).await {
            Ok(_) => {
                let payload_len = u32::from_be_bytes(len_buf) as usize;
                if payload_len > super::protocol::MAX_FRAME_SIZE as usize {
                    warn!("DERP recv_task: frame too large ({} > max)", payload_len);
                    break;
                }
                let mut payload_buf = vec![0u8; payload_len];
                match tokio::io::AsyncReadExt::read_exact(&mut read_half, &mut payload_buf).await {
                    Ok(_) => {
                        let mut combined = BytesMut::from(&len_buf[..]);
                        combined.extend_from_slice(&payload_buf);
                        match Frame::decode(&mut combined) {
                            Ok(Some((frame, _))) => match frame {
                                Frame::RecvPacket { payload, .. } => {
                                    trace!("DERP recv_task: forwarding {} bytes", payload.len());
                                    if read_tx.send(payload.into()).is_err() {
                                        debug!("DERP recv_task: read_tx closed");
                                        break;
                                    }
                                }
                                Frame::KeepAlive => {
                                    trace!("DERP recv_task: keepalive received");
                                }
                                other => {
                                    trace!("DERP recv_task: ignoring frame {:?}", other);
                                }
                            },
                            Ok(None) => {
                                // 不应该发生，因为我们已经读取了完整长度
                                warn!("DERP recv_task: incomplete frame after exact read");
                            }
                            Err(e) => {
                                warn!("DERP recv_task: decode error: {}", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        if e.kind() == io::ErrorKind::UnexpectedEof {
                            debug!("DERP recv_task: peer closed connection");
                        } else {
                            debug!("DERP recv_task: payload read error: {}", e);
                        }
                        break;
                    }
                }
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    debug!("DERP recv_task: peer closed connection");
                } else {
                    debug!("DERP recv_task: length read error: {}", e);
                }
                break;
            }
        }
    }
    debug!("DERP recv_task exited");
}

/// 通过 DERP 中继传输的可靠字节管道。
///
/// 将上层连续字节流切分为 DERP `SendPacket` 帧发送，
/// 并将收到的 `RecvPacket` 帧 payload 拼接为连续字节流供上层读取。
#[derive(Debug)]
pub struct DerpPipe {
    /// 目标设备 ID（用于 SendPacket 路由）
    target: DeviceId,
    /// 本地设备 ID
    local_device_id: DeviceId,
    /// 发送任务句柄
    send_handle: Option<tokio::task::JoinHandle<()>>,
    /// 写入帧数据的 channel（shutdown 时替换为 closed channel）
    write_tx: mpsc::UnboundedSender<Vec<u8>>,
    /// 从 recv_task 接收数据的 channel
    read_rx: mpsc::UnboundedReceiver<Bytes>,
    /// 当前读取缓冲区（从 read_rx 收到的 Bytes 的剩余部分）
    read_buffer: Bytes,
    /// 本地地址
    local_addr: Option<SocketAddr>,
    /// 对端地址
    peer_addr: Option<SocketAddr>,
}

impl DerpPipe {
    /// 创建新的 DerpPipe。
    ///
    /// # Arguments
    /// * `stream` - 已连接到 DERP 服务器的 TCP stream
    /// * `target` - 目标设备 ID（SendPacket 的路由目标）
    /// * `local_device_id` - 本地设备 ID
    pub fn new(
        stream: tokio::net::TcpStream,
        target: DeviceId,
        local_device_id: DeviceId,
    ) -> Self {
        let local_addr = stream.local_addr().ok();
        let peer_addr = stream.peer_addr().ok();
        let (read_half, write_half) = stream.into_split();

        let (read_tx, read_rx) = mpsc::unbounded_channel::<Bytes>();
        let (write_tx, write_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        let _recv_handle = tokio::spawn(recv_task(read_half, read_tx));
        let send_handle = tokio::spawn(send_task(write_half, write_rx));

        // 当 DerpPipe drop 时，我们需要关闭 send_handle 和 recv_handle
        // 这里我们保留 send_handle 以便在 drop 时 abort
        // recv_handle 不需要保留，因为它会在 read_tx 被 drop 后自动退出

        Self {
            target,
            local_device_id,
            send_handle: Some(send_handle),
            write_tx,
            read_rx,
            read_buffer: Bytes::new(),
            local_addr,
            peer_addr,
        }
    }

    /// 获取目标设备 ID
    pub fn target(&self) -> &DeviceId {
        &self.target
    }

    /// 获取本地设备 ID
    pub fn local_device_id(&self) -> &DeviceId {
        &self.local_device_id
    }
}

impl AsyncRead for DerpPipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // 1. 先消耗 read_buffer 中的剩余数据
        if !self.read_buffer.is_empty() {
            let to_copy = std::cmp::min(buf.remaining(), self.read_buffer.len());
            buf.put_slice(&self.read_buffer[..to_copy]);
            self.read_buffer.advance(to_copy);
            return Poll::Ready(Ok(()));
        }

        // 2. 从 read_rx 接收下一个数据块
        match self.read_rx.poll_recv(cx) {
            Poll::Ready(Some(mut bytes)) => {
                let to_copy = std::cmp::min(buf.remaining(), bytes.len());
                buf.put_slice(&bytes[..to_copy]);
                if to_copy < bytes.len() {
                    self.read_buffer = bytes.split_off(to_copy);
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => {
                // Channel 已关闭，返回 EOF
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for DerpPipe {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        // 将 buf 编码为 SendPacket 帧
        let frame = Frame::SendPacket {
            target: this.target,
            payload: buf.to_vec(),
        };

        let encoded = frame.encode();
        match this.write_tx.send(encoded) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(_) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "DERP send task closed",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // 数据已发送到 channel，flush 由 send_task 负责
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        // 关闭 write_tx，这会触发 send_task 退出
        let (closed_tx, _) = mpsc::unbounded_channel::<Vec<u8>>();
        let _ = std::mem::replace(&mut this.write_tx, closed_tx);
        Poll::Ready(Ok(()))
    }
}

impl ReliablePipe for DerpPipe {
    fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Relay
    }

    fn path_quality(&self) -> PathQuality {
        // DERP relay 的路径质量通常比直接连接差
        PathQuality {
            rtt: std::time::Duration::from_millis(150),
            packet_loss: 0.0,
            estimated_bps: None,
            last_updated: std::time::Instant::now(),
        }
    }
}

impl Drop for DerpPipe {
    fn drop(&mut self) {
        // 关闭 channel，让后台任务退出
        let (closed_tx, _) = mpsc::unbounded_channel::<Vec<u8>>();
        let _ = std::mem::replace(&mut self.write_tx, closed_tx);
        // abort send_task
        if let Some(handle) = self.send_handle.take() {
            handle.abort();
        }
    }
}
