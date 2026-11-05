//! DERP Transport: implements the DERP relay protocol as a `Transport`.
//!
//! Allows `ConnectionManager` to establish BEP connections through a DERP relay.
//!
//! ## Usage
//! ```rust,ignore
//! let transport = DerpTransport::new(local_device_id);
//! let pipe = transport.dial_device(derp_server_addr, &target_device_id).await?;
//! // pipe can be used directly for the TLS + BEP handshake
//! ```

use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info, warn};

use syncthing_core::{
    BoxedPipe, DeviceId, Result as CoreResult, SyncthingError, Transport, TransportListener,
};

use super::pipe::DerpPipe;
use super::protocol::{Frame, PROTOCOL_VERSION};
use super::server::{DerpServer, DerpServerConfig};

/// DERP transport implementation.
///
/// `dial_device(addr, device_id)` connects to the DERP server at `addr`;
/// after registering the local device, subsequent writes are routed to `device_id`.
#[derive(Debug)]
pub struct DerpTransport {
    local_device_id: DeviceId,
}

impl DerpTransport {
    /// Creates a new DERP transport.
    pub fn new(local_device_id: DeviceId) -> Self {
        Self { local_device_id }
    }

    /// Returns the local device ID.
    pub fn local_device_id(&self) -> &DeviceId {
        &self.local_device_id
    }

    /// Connects to the DERP server and completes the handshake.
    ///
    /// Returns the handshaken TCP stream for DerpPipe to wrap.
    async fn connect_server(&self, server_addr: SocketAddr) -> CoreResult<tokio::net::TcpStream> {
        debug!("DERP connecting to server at {}", server_addr);

        let mut stream = tokio::net::TcpStream::connect(server_addr)
            .await
            .map_err(|e| SyncthingError::connection(format!("DERP connect failed: {}", e)))?;

        let client_info = Frame::ClientInfo {
            device_id: self.local_device_id,
            version: PROTOCOL_VERSION,
        };
        let encoded = client_info.encode();
        stream.write_all(&encoded).await.map_err(|e| {
            SyncthingError::connection(format!("DERP ClientInfo send failed: {}", e))
        })?;

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.map_err(|e| {
            SyncthingError::connection(format!("DERP ServerInfo read failed: {}", e))
        })?;
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        stream.read_exact(&mut payload_buf).await.map_err(|e| {
            SyncthingError::connection(format!("DERP ServerInfo payload read failed: {}", e))
        })?;

        let mut combined = bytes::BytesMut::from(&len_buf[..]);
        combined.extend_from_slice(&payload_buf);
        let (frame, _) = Frame::decode(&mut combined)
            .map_err(|e| SyncthingError::protocol(format!("DERP ServerInfo decode failed: {}", e)))?
            .ok_or_else(|| SyncthingError::protocol("DERP ServerInfo incomplete"))?;

        match frame {
            Frame::ServerInfo { version } => {
                info!(
                    "DERP handshake complete with {} (server version: {})",
                    server_addr, version
                );
            }
            other => {
                return Err(SyncthingError::protocol(format!(
                    "DERP unexpected server response: {:?}",
                    other
                )));
            }
        }

        Ok(stream)
    }
}

#[async_trait::async_trait]
impl Transport for DerpTransport {
    fn scheme(&self) -> &'static str {
        "derp"
    }

    /// Starts a DERP server listener (for tests or self-hosted relays; production
    /// clients usually connect to public DERP servers).
    async fn bind(&self, addr: SocketAddr) -> CoreResult<Box<dyn TransportListener>> {
        let mut server = DerpServer::new(DerpServerConfig {
            bind_addr: addr,
            ..Default::default()
        });
        let actual_addr = server.bind().await?;

        info!("DERP server starting on {}", actual_addr);

        tokio::spawn(async move {
            if let Err(e) = server.run().await {
                warn!("DERP server exited with error: {}", e);
            }
        });

        // Stub listener: DerpServer runs its own accept loop, so this only
        // carries the bound address.
        let listener = DerpTransportListener { addr: actual_addr };
        Ok(Box::new(listener))
    }

    /// Plain dial is not supported because a target device ID is required; use `dial_device`.
    async fn dial(&self, _addr: SocketAddr) -> CoreResult<BoxedPipe> {
        Err(SyncthingError::config(
            "DerpTransport::dial() is not supported; use dial_device() instead",
        ))
    }

    /// Dials the target device through a DERP relay.
    ///
    /// `addr` is the DERP server address; `device_id` is the target device.
    async fn dial_device(&self, addr: SocketAddr, device_id: &DeviceId) -> CoreResult<BoxedPipe> {
        let stream = self.connect_server(addr).await?;
        let pipe = DerpPipe::new(stream, *device_id, self.local_device_id);
        Ok(Box::new(pipe))
    }
}

/// Stub DERP TransportListener.
///
/// DerpServer runs its own accept loop, so this listener only provides the
/// bound address; incoming connections are managed directly by DerpServer.
#[derive(Debug)]
struct DerpTransportListener {
    addr: SocketAddr,
}

#[async_trait::async_trait]
impl TransportListener for DerpTransportListener {
    async fn accept(&self) -> CoreResult<(BoxedPipe, SocketAddr)> {
        // Traditional accept is not supported; incoming DERP connections are
        // handled by DerpServer's internal loop.
        Err(SyncthingError::config(
            "DerpTransportListener::accept() not supported; use DerpServer directly",
        ))
    }

    fn local_addr(&self) -> CoreResult<SocketAddr> {
        Ok(self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn test_derp_transport_dial_device_and_exchange() {
        // 1. 启动 DERP 服务器
        let mut server = DerpServer::new(DerpServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        });
        let server_addr = server.bind().await.unwrap();
        tokio::spawn(async move {
            let _ = server.run().await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let device_a = DeviceId::from_bytes(&[1u8; 32]).unwrap();
        let device_b = DeviceId::from_bytes(&[2u8; 32]).unwrap();

        // 2. A 和 B 分别连接 DERP 服务器
        let transport_a = DerpTransport::new(device_a);
        let transport_b = DerpTransport::new(device_b);

        let mut pipe_a = transport_a
            .dial_device(server_addr, &device_b)
            .await
            .unwrap();
        let mut pipe_b = transport_b
            .dial_device(server_addr, &device_a)
            .await
            .unwrap();

        // 3. A 写入数据
        pipe_a.write_all(b"hello from A").await.unwrap();

        // 4. B 读取数据
        let mut buf = [0u8; 64];
        let n = pipe_b.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello from A");

        // 5. B 写入数据
        pipe_b.write_all(b"hello from B").await.unwrap();

        // 6. A 读取数据
        let n = pipe_a.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello from B");
    }

    #[tokio::test]
    async fn test_derp_transport_dial_without_device_id_fails() {
        let device_a = DeviceId::from_bytes(&[1u8; 32]).unwrap();
        let transport = DerpTransport::new(device_a);

        let result = transport.dial("127.0.0.1:3478".parse().unwrap()).await;
        match result {
            Err(e) => {
                let err_msg = format!("{}", e);
                assert!(err_msg.contains("dial_device"));
            }
            Ok(_) => panic!("expected dial() to fail"),
        }
    }
}
