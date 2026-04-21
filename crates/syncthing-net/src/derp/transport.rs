//! DERP Transport：将 DERP 中继协议实现为 `Transport` trait。
//!
//! 使 `ConnectionManager` 可以通过 DERP relay 建立 BEP 连接。
//!
//! ## 使用方式
//! ```rust,ignore
//! let transport = DerpTransport::new(local_device_id);
//! let pipe = transport.dial_device(derp_server_addr, &target_device_id).await?;
//! // pipe 可直接用于 TLS + BEP 握手
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

/// DERP 传输实现。
///
/// `dial_device(addr, device_id)` 连接到 DERP 服务器 `addr`，
/// 注册本地设备后，后续写入的数据将自动路由到 `device_id`。
#[derive(Debug)]
pub struct DerpTransport {
    local_device_id: DeviceId,
}

impl DerpTransport {
    /// 创建新的 DERP 传输。
    pub fn new(local_device_id: DeviceId) -> Self {
        Self { local_device_id }
    }

    /// 获取本地设备 ID。
    pub fn local_device_id(&self) -> &DeviceId {
        &self.local_device_id
    }

    /// 连接到 DERP 服务器并完成握手。
    ///
    /// 返回已握手的 TCP stream，供 DerpPipe 包装。
    async fn connect_server(&self, server_addr: SocketAddr) -> CoreResult<tokio::net::TcpStream> {
        debug!("DERP connecting to server at {}", server_addr);

        let mut stream = tokio::net::TcpStream::connect(server_addr)
            .await
            .map_err(|e| SyncthingError::connection(format!("DERP connect failed: {}", e)))?;

        // 发送 ClientInfo
        let client_info = Frame::ClientInfo {
            device_id: self.local_device_id,
            version: PROTOCOL_VERSION,
        };
        let encoded = client_info.encode();
        stream
            .write_all(&encoded)
            .await
            .map_err(|e| SyncthingError::connection(format!("DERP ClientInfo send failed: {}", e)))?;

        // 读取 ServerInfo
        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| {
                SyncthingError::connection(format!("DERP ServerInfo read failed: {}", e))
            })?;
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload_buf = vec![0u8; payload_len];
        stream
            .read_exact(&mut payload_buf)
            .await
            .map_err(|e| {
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

    /// 启动 DERP 服务器监听。
    ///
    /// 注意：这是用于测试或自建 relay 的场景。生产环境中通常连接到公共 DERP 服务器。
    async fn bind(&self, addr: SocketAddr) -> CoreResult<Box<dyn TransportListener>> {
        let mut server = DerpServer::new(DerpServerConfig {
            bind_addr: addr,
            ..Default::default()
        });
        let actual_addr = server.bind().await?;

        info!("DERP server starting on {}", actual_addr);

        // 在后台运行服务器
        tokio::spawn(async move {
            if let Err(e) = server.run().await {
                warn!("DERP server exited with error: {}", e);
            }
        });

        // 返回一个简化的 TransportListener
        // 由于 DerpServer 内部自行 accept，这里返回一个 stub listener
        // 实际生产中客户端不需要 bind DERP 服务器
        let listener = DerpTransportListener { addr: actual_addr };
        Ok(Box::new(listener))
    }

    /// 普通拨号（不支持，因为需要 device_id）。
    ///
    /// 请使用 `dial_device`。
    async fn dial(&self, _addr: SocketAddr) -> CoreResult<BoxedPipe> {
        Err(SyncthingError::config(
            "DerpTransport::dial() is not supported; use dial_device() instead",
        ))
    }

    /// 向指定设备拨号，通过 DERP 中继。
    ///
    /// `addr` 是 DERP 服务器地址，`device_id` 是目标设备。
    async fn dial_device(
        &self,
        addr: SocketAddr,
        device_id: &DeviceId,
    ) -> CoreResult<BoxedPipe> {
        let stream = self.connect_server(addr).await?;
        let pipe = DerpPipe::new(stream, *device_id, self.local_device_id);
        Ok(Box::new(pipe))
    }
}

/// DERP TransportListener stub。
///
/// 由于 DerpServer 内部自行处理 accept 循环，此 listener 仅提供地址信息。
/// 实际的传入连接由 DerpServer 直接管理。
#[derive(Debug)]
struct DerpTransportListener {
    addr: SocketAddr,
}

#[async_trait::async_trait]
impl TransportListener for DerpTransportListener {
    async fn accept(&self) -> CoreResult<(BoxedPipe, SocketAddr)> {
        // DerpTransportListener 不支持传统 accept。
        // 传入的 DERP 连接由 DerpServer 内部循环处理。
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
