//! Relay 协议客户端
//!
//! 实现 Protocol Mode（TLS）和 Session Mode（明文）两种连接方式。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tracing::{debug, info, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

use crate::relay::protocol::{
    read_message, write_message, ConnectRequest, JoinRelayRequest, JoinSessionRequest, Message,
    Ping, SessionInvitation,
};

/// Protocol Mode 下的 Relay 连接
///
/// 通过 TLS 与中继服务器通信，支持永久模式（JoinRelay + 等待邀请）
/// 和临时模式（ConnectRequest + 获取邀请）。
pub struct RelayProtocolClient {
    stream: tokio_rustls::client::TlsStream<TcpStream>,
}

impl RelayProtocolClient {
    /// 以 Protocol Mode 连接 relay 服务器
    ///
    /// # 参数
    /// - `addr`: relay 服务器地址（通常为 `:22067`）
    /// - `tls_config`: 用于 `bep-relay` 协议的 TLS 客户端配置
    pub async fn connect(
        addr: SocketAddr,
        tls_config: &Arc<rustls::ClientConfig>,
    ) -> Result<Self> {
        let tcp_stream = TcpStream::connect(addr)
            .await
            .map_err(|e| SyncthingError::network(format!("relay tcp connect: {}", e)))?;

        let connector = TlsConnector::from(Arc::clone(tls_config));
        let server_name = rustls::pki_types::ServerName::try_from(addr.ip().to_string())
            .map_err(|e| SyncthingError::Tls(format!("invalid server name: {}", e)))?;
        let tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .map_err(|e| SyncthingError::Tls(format!("relay tls handshake: {}", e)))?;

        debug!("Relay protocol mode connected to {}", addr);
        Ok(Self { stream: tls_stream })
    }

    /// 注册到 relay 服务器（永久模式）
    ///
    /// 发送 JoinRelayRequest，等待 ResponseSuccess。
    pub async fn join_relay(&mut self) -> Result<()> {
        write_message(
            &mut self.stream,
            &Message::JoinRelayRequest(JoinRelayRequest),
        )
        .await?;

        match timeout(Duration::from_secs(30), read_message(&mut self.stream))
            .await
            .map_err(|_| SyncthingError::network("relay join timeout"))?
        {
            Ok(Message::Response(resp)) if resp.code == 0 => {
                info!("Relay join success: {}", resp.message);
                Ok(())
            }
            Ok(Message::Response(resp)) if resp.code == 2 => {
                warn!("Relay join already connected");
                Ok(()) // 同设备 ID 已连接，视为成功
            }
            Ok(Message::Response(resp)) => Err(SyncthingError::network(format!(
                "relay join rejected: {} (code {})",
                resp.message, resp.code
            ))),
            Ok(Message::RelayFull(_)) => {
                Err(SyncthingError::network("relay full".to_string()))
            }
            Ok(other) => Err(SyncthingError::network(format!(
                "unexpected message during relay join: {:?}",
                other
            ))),
            Err(e) => Err(e.into()),
        }
    }

    /// 请求与目标设备建立中继会话（临时模式）
    ///
    /// 发送 ConnectRequest，等待 SessionInvitation 或错误响应。
    pub async fn request_session(
        &mut self,
        target: DeviceId,
    ) -> Result<SessionInvitation> {
        let req = ConnectRequest {
            id: target.0.to_vec(),
        };
        write_message(&mut self.stream, &Message::ConnectRequest(req)).await?;

        match timeout(Duration::from_secs(60), read_message(&mut self.stream))
            .await
            .map_err(|_| SyncthingError::network("relay session request timeout"))?
        {
            Ok(Message::SessionInvitation(inv)) => {
                info!(
                    "Relay session invitation received: from={:?}, addr={}:{}, server_socket={}",
                    inv.from,
                    String::from_utf8_lossy(&inv.address),
                    inv.port,
                    inv.server_socket
                );
                Ok(inv)
            }
            Ok(Message::Response(resp)) if resp.code == 1 => Err(SyncthingError::network(
                "peer not found on relay".to_string(),
            )),
            Ok(Message::RelayFull(_)) => {
                Err(SyncthingError::network("relay full".to_string()))
            }
            Ok(Message::Response(resp)) => Err(SyncthingError::network(format!(
                "relay connect rejected: {} (code {})",
                resp.message, resp.code
            ))),
            Ok(other) => Err(SyncthingError::network(format!(
                "unexpected message during relay connect: {:?}",
                other
            ))),
            Err(e) => Err(e.into()),
        }
    }

    /// 等待来自 relay 的 SessionInvitation（被动模式）
    ///
    /// 永久模式下，relay 会在其他设备请求连接时转发 SessionInvitation。
    pub async fn wait_invitation(&mut self) -> Result<SessionInvitation> {
        loop {
            match read_message(&mut self.stream).await {
                Ok(Message::SessionInvitation(inv)) => {
                    info!(
                        "Relay session invitation received (passive): from={:?}, addr={}:{}, server_socket={}",
                        inv.from,
                        String::from_utf8_lossy(&inv.address),
                        inv.port,
                        inv.server_socket
                    );
                    return Ok(inv);
                }
                Ok(Message::Ping(_)) => {
                    // relay 发送的 Ping，回复 Pong 后继续等待
                    write_message(&mut self.stream, &Message::Pong(super::protocol::Pong)).await?;
                    continue;
                }
                Ok(other) => {
                    return Err(SyncthingError::network(format!(
                        "unexpected message while waiting invitation: {:?}",
                        other
                    )))
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// 发送 Ping 并等待 Pong
    pub async fn ping(&mut self) -> Result<()> {
        write_message(&mut self.stream, &Message::Ping(Ping)).await?;
        match timeout(Duration::from_secs(10), read_message(&mut self.stream))
            .await
            .map_err(|_| SyncthingError::network("relay ping timeout"))?
        {
            Ok(Message::Pong(_)) => Ok(()),
            Ok(other) => Err(SyncthingError::network(format!(
                "unexpected message during ping: {:?}",
                other
            ))),
            Err(e) => Err(e.into()),
        }
    }

    /// 获取底层 stream 的本地地址
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.stream.get_ref().0.local_addr()
    }

    /// 获取底层 stream 的对端地址
    pub fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        self.stream.get_ref().0.peer_addr()
    }
}

/// Session Mode 连接辅助函数
///
/// 在获取 `SessionInvitation` 后，以明文 TCP 连接到 relay 的 session 端口，
/// 发送 `JoinSessionRequest`，成功后返回可用于 BEP TLS 握手的 `TcpStream`。
pub async fn join_session(
    addr: SocketAddr,
    key: &[u8],
) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| SyncthingError::network(format!("session mode connect: {}", e)))?;

    debug!("Session mode connected to {}, sending JoinSessionRequest", addr);

    let req = JoinSessionRequest {
        key: key.to_vec(),
    };
    write_message(&mut stream, &Message::JoinSessionRequest(req))
        .await
        .map_err(|e| SyncthingError::network(format!("write JoinSessionRequest: {}", e)))?;

    match timeout(Duration::from_secs(30), read_message(&mut stream))
        .await
        .map_err(|_| SyncthingError::network("session join timeout"))?
    {
        Ok(Message::Response(resp)) if resp.code == 0 => {
            info!("Session joined successfully on {}", addr);
            Ok(stream)
        }
        Ok(Message::Response(resp)) if resp.code == 1 => Err(SyncthingError::network(
            "session key not found".to_string(),
        )),
        Ok(Message::Response(resp)) if resp.code == 2 => Err(SyncthingError::network(
            "session already full".to_string(),
        )),
        Ok(Message::RelayFull(_)) => {
            Err(SyncthingError::network("relay full in session mode".to_string()))
        }
        Ok(Message::Response(resp)) => Err(SyncthingError::network(format!(
            "session join rejected: {} (code {})",
            resp.message, resp.code
        ))),
        Ok(other) => Err(SyncthingError::network(format!(
            "unexpected message in session mode: {:?}",
            other
        ))),
        Err(e) => Err(e.into()),
    }
}
