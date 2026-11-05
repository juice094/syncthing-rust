//! DERP Pipe: wraps DERP message forwarding as a `ReliablePipe` byte stream.
//!
//! DERP is a message-based forwarding protocol (SendPacket / RecvPacket).
//! To adapt it to BEP (which needs a reliable byte stream), received frame
//! payloads are concatenated into a continuous byte stream, and writes are
//! automatically chunked into SendPacket frames.

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

use syncthing_core::DeviceId;
use syncthing_core::{PathQuality, ReliablePipe, TransportType};

use super::protocol::Frame;

/// Internal send task: writes encoded frame data from the channel to the TCP stream.
async fn send_task(mut write_half: OwnedWriteHalf, mut rx: mpsc::Receiver<Vec<u8>>) {
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

/// Internal receive task: reads frames from the TCP stream and forwards
/// RecvPacket payloads to the read end.
async fn recv_task(mut read_half: OwnedReadHalf, read_tx: mpsc::Sender<Bytes>) {
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
                                    if read_tx.try_send(payload.into()).is_err() {
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

/// Reliable byte pipe over a DERP relay.
///
/// Chunks the upper-layer byte stream into DERP `SendPacket` frames on write
/// and concatenates received `RecvPacket` payloads into a continuous byte
/// stream on read.
#[derive(Debug)]
pub struct DerpPipe {
    /// Target device ID (SendPacket routing target).
    target: DeviceId,
    /// Local device ID.
    local_device_id: DeviceId,
    /// Send task handle.
    send_handle: Option<tokio::task::JoinHandle<()>>,
    /// Channel for frame data written by `poll_write` (swapped with a closed
    /// channel on shutdown).
    write_tx: mpsc::Sender<Vec<u8>>,
    /// Channel receiving data from recv_task.
    read_rx: mpsc::Receiver<Bytes>,
    /// Current read buffer (remainder of the last Bytes from read_rx).
    read_buffer: Bytes,
    /// Local address.
    local_addr: Option<SocketAddr>,
    /// Peer address.
    peer_addr: Option<SocketAddr>,
}

impl DerpPipe {
    /// Creates a new DerpPipe over a TCP stream already connected to a DERP server.
    ///
    /// * `target` - target device ID (SendPacket routing target)
    /// * `local_device_id` - local device ID
    pub fn new(stream: tokio::net::TcpStream, target: DeviceId, local_device_id: DeviceId) -> Self {
        let local_addr = stream.local_addr().ok();
        let peer_addr = stream.peer_addr().ok();
        let (read_half, write_half) = stream.into_split();

        let (read_tx, read_rx) = mpsc::channel::<Bytes>(1024);
        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(1024);

        let _recv_handle = tokio::spawn(recv_task(read_half, read_tx));
        let send_handle = tokio::spawn(send_task(write_half, write_rx));

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

    /// Returns the target device ID.
    pub fn target(&self) -> &DeviceId {
        &self.target
    }

    /// Returns the local device ID.
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
        if !self.read_buffer.is_empty() {
            let to_copy = std::cmp::min(buf.remaining(), self.read_buffer.len());
            buf.put_slice(&self.read_buffer[..to_copy]);
            self.read_buffer.advance(to_copy);
            return Poll::Ready(Ok(()));
        }

        match self.read_rx.poll_recv(cx) {
            Poll::Ready(Some(mut bytes)) => {
                let to_copy = std::cmp::min(buf.remaining(), bytes.len());
                buf.put_slice(&bytes[..to_copy]);
                if to_copy < bytes.len() {
                    self.read_buffer = bytes.split_off(to_copy);
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Ok(())),
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

        let frame = Frame::SendPacket {
            target: this.target,
            payload: buf.to_vec(),
        };

        let encoded = frame.encode();
        match this.write_tx.try_send(encoded) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(_) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "DERP send task closed",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Data is handed to the channel; flushing is send_task's job.
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        // Closing write_tx makes send_task exit.
        let (closed_tx, _) = mpsc::channel::<Vec<u8>>(1);
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
        // A DERP relay path is typically worse than a direct connection.
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
        // Close the channel so background tasks exit; recv_task also exits
        // when read_tx is dropped.
        let (closed_tx, _) = mpsc::channel::<Vec<u8>>(1);
        let _ = std::mem::replace(&mut self.write_tx, closed_tx);
        if let Some(handle) = self.send_handle.take() {
            handle.abort();
        }
    }
}
