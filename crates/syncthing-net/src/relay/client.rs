//! Relay protocol client.
//!
//! Implements both Protocol Mode (TLS) and Session Mode (plaintext) connections.

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

/// Relay connection in Protocol Mode.
///
/// Communicates with the relay server over TLS, supporting permanent mode
/// (JoinRelay + wait for invitations) and temporary mode (ConnectRequest +
/// receive an invitation).
pub struct RelayProtocolClient {
    stream: tokio_rustls::client::TlsStream<TcpStream>,
}

impl RelayProtocolClient {
    /// Connects to a relay server in Protocol Mode.
    ///
    /// * `addr` - relay server address (usually `:22067`)
    /// * `tls_config` - TLS client config for the `bep-relay` protocol
    ///
    /// # Errors
    /// Returns an error on TCP connect failure, invalid server name, or TLS
    /// handshake failure.
    pub async fn connect(addr: SocketAddr, tls_config: &Arc<rustls::ClientConfig>) -> Result<Self> {
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

    /// Registers with the relay server (permanent mode).
    ///
    /// Sends JoinRelayRequest and waits for a success response.
    ///
    /// # Errors
    /// Returns an error on timeout, rejection, or relay full.
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
                // Same device ID already connected; treat as success.
                Ok(())
            }
            Ok(Message::Response(resp)) => Err(SyncthingError::network(format!(
                "relay join rejected: {} (code {})",
                resp.message, resp.code
            ))),
            Ok(Message::RelayFull(_)) => Err(SyncthingError::network("relay full".to_string())),
            Ok(other) => Err(SyncthingError::network(format!(
                "unexpected message during relay join: {:?}",
                other
            ))),
            Err(e) => Err(e.into()),
        }
    }

    /// Requests a relay session with the target device (temporary mode).
    ///
    /// Sends ConnectRequest and waits for a SessionInvitation or an error response.
    ///
    /// # Errors
    /// Returns an error on timeout, peer not found, rejection, or relay full.
    pub async fn request_session(&mut self, target: DeviceId) -> Result<SessionInvitation> {
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
            Ok(Message::RelayFull(_)) => Err(SyncthingError::network("relay full".to_string())),
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

    /// Waits for a SessionInvitation from the relay (passive mode).
    ///
    /// In permanent mode the relay forwards a SessionInvitation whenever
    /// another device requests a connection.
    ///
    /// # Errors
    /// Returns an error on unexpected messages or connection failure.
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
                    // Answer relay keepalive pings with Pong and keep waiting.
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

    /// Sends a Ping and waits for the Pong.
    ///
    /// # Errors
    /// Returns an error on timeout or an unexpected response.
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

    /// Returns the local address of the underlying stream.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.stream.get_ref().0.local_addr()
    }

    /// Returns the peer address of the underlying stream.
    pub fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        self.stream.get_ref().0.peer_addr()
    }
}

/// Session Mode connection helper.
///
/// After receiving a `SessionInvitation`, connects in plaintext TCP to the
/// relay's session port and sends `JoinSessionRequest`; on success returns a
/// `TcpStream` ready for the BEP TLS handshake.
///
/// # Errors
/// Returns an error on connect failure, timeout, key not found, session full,
/// or rejection.
pub async fn join_session(addr: SocketAddr, key: &[u8]) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| SyncthingError::network(format!("session mode connect: {}", e)))?;

    debug!(
        "Session mode connected to {}, sending JoinSessionRequest",
        addr
    );

    let req = JoinSessionRequest { key: key.to_vec() };
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
        Ok(Message::Response(resp)) if resp.code == 1 => {
            Err(SyncthingError::network("session key not found".to_string()))
        }
        Ok(Message::Response(resp)) if resp.code == 2 => {
            Err(SyncthingError::network("session already full".to_string()))
        }
        Ok(Message::RelayFull(_)) => Err(SyncthingError::network(
            "relay full in session mode".to_string(),
        )),
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
