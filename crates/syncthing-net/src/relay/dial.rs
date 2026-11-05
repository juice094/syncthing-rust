//! Relay dialing helpers.
//!
//! Bridges the Relay Protocol with the BEP connection layer, providing
//! `connect_bep_via_relay`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;
use tracing::{debug, info};

use syncthing_core::{ConnectionType, DeviceId, Result, SyncthingError};

use crate::connection::BepConnection;
use crate::connection::TcpBiStream;
use crate::handshaker::BepHandshaker;
use crate::relay::client::{join_session, RelayProtocolClient};
use crate::relay::protocol::SessionInvitation;
use crate::tls::{accept_tls_stream, connect_tls_stream, SyncthingTlsConfig};

/// Establishes a BEP connection to the target device through a relay server.
///
/// Full flow:
/// 1. Protocol Mode TLS to the relay server (ALPN = `bep-relay`)
/// 2. Send `ConnectRequest(target_device)`
/// 3. Receive `SessionInvitation`
/// 4. Session Mode TCP to the relay's session port
/// 5. Send `JoinSessionRequest(key)`
/// 6. Start BEP TLS on the session stream (client/server decided by `server_socket`)
/// 7. BEP Hello exchange
/// 8. Create the `BepConnection`
///
/// # Errors
/// Returns an error on any step failing (see per-step timeouts), on device ID
/// mismatch, or on SSRF prevention rejecting the session address.
pub async fn connect_bep_via_relay(
    relay_url: &str,
    target_device: DeviceId,
    device_name: &str,
    tls_config: &Arc<SyncthingTlsConfig>,
) -> Result<Arc<BepConnection>> {
    let (relay_addr, _relay_id) = parse_relay_url(relay_url)?;

    let rustls_config = tls_config
        .relay_client_config()
        .map_err(|e| SyncthingError::Tls(format!("relay client config: {}", e)))?;
    let rustls_config = Arc::new(rustls_config);

    let mut protocol_client = timeout(
        Duration::from_secs(30),
        RelayProtocolClient::connect(relay_addr, &rustls_config),
    )
    .await
    .map_err(|_| SyncthingError::timeout("relay protocol connect timeout"))??;

    debug!("Relay protocol mode connected to {}", relay_addr);

    let invitation = timeout(
        Duration::from_secs(60),
        protocol_client.request_session(target_device),
    )
    .await
    .map_err(|_| SyncthingError::timeout("relay session request timeout"))??;

    // Close the protocol connection explicitly: the dialer only needs the
    // invitation and should not hold a protocol-mode slot against the listener.
    drop(protocol_client);

    info!(
        "Relay session invitation: {}:{} (server_socket={})",
        String::from_utf8_lossy(&invitation.address),
        invitation.port,
        invitation.server_socket
    );

    let session_addr = resolve_session_addr(relay_addr, &invitation)?;
    let session_stream = timeout(
        Duration::from_secs(30),
        join_session(session_addr, &invitation.key),
    )
    .await
    .map_err(|_| SyncthingError::timeout("relay session join timeout"))??;

    crate::transport::keepalive::apply_tcp_keepalive(
        &session_stream,
        syncthing_core::TransportType::Relay,
    );

    debug!("Relay session mode connected to {}", session_addr);

    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

    let conn = if invitation.server_socket {
        let (mut tls_stream, peer_device) = timeout(
            Duration::from_secs(30),
            accept_tls_stream(session_stream, tls_config),
        )
        .await
        .map_err(|_| SyncthingError::timeout("relay TLS accept timeout"))??;

        if peer_device != target_device {
            return Err(SyncthingError::Tls(format!(
                "relay peer device ID mismatch: expected {}, got {}",
                target_device, peer_device
            )));
        }

        debug!(
            "Relay BEP TLS handshake completed with {} (server)",
            peer_device
        );

        let _remote_hello = BepHandshaker::server_handshake(&mut tls_stream, device_name).await?;

        BepConnection::new(
            Box::new(TcpBiStream::relay_server(tls_stream)),
            ConnectionType::Outgoing,
            event_tx,
        )
        .await?
    } else {
        let (mut tls_stream, peer_device) = timeout(
            Duration::from_secs(30),
            connect_tls_stream(session_stream, tls_config, Some(target_device)),
        )
        .await
        .map_err(|_| SyncthingError::timeout("relay TLS connect timeout"))??;

        if peer_device != target_device {
            return Err(SyncthingError::Tls(format!(
                "relay peer device ID mismatch: expected {}, got {}",
                target_device, peer_device
            )));
        }

        debug!(
            "Relay BEP TLS handshake completed with {} (client)",
            peer_device
        );

        let _remote_hello = BepHandshaker::client_handshake(&mut tls_stream, device_name).await?;

        BepConnection::new(
            Box::new(TcpBiStream::relay_client(tls_stream)),
            ConnectionType::Outgoing,
            event_tx,
        )
        .await?
    };

    conn.set_device_id(target_device);
    conn.set_state(syncthing_core::ConnectionState::ProtocolHandshakeComplete);

    info!(
        "Relay BEP connection established to {} via {}",
        target_device, relay_addr
    );

    Ok(conn)
}

/// Parses a relay URL of the form `relay://host:port/?id=<device_id>`.
///
/// # Errors
/// Returns an error on a missing `relay://` prefix, an invalid address, or an
/// invalid device ID.
pub fn parse_relay_url(url: &str) -> Result<(SocketAddr, DeviceId)> {
    let url = url
        .strip_prefix("relay://")
        .ok_or_else(|| SyncthingError::config("relay URL must start with relay://"))?;

    let (addr_part, id_part) = url
        .split_once("/?id=")
        .or_else(|| url.split_once("?id="))
        .unwrap_or((url, ""));

    let addr: SocketAddr = addr_part
        .parse()
        .map_err(|e| SyncthingError::config(format!("invalid relay address: {}", e)))?;

    let relay_id = if id_part.is_empty() {
        DeviceId::default()
    } else {
        id_part
            .parse::<DeviceId>()
            .map_err(|e| SyncthingError::config(format!("invalid relay device ID: {}", e)))?
    };

    Ok((addr, relay_id))
}

/// Resolves the session address.
///
/// Falls back to the protocol-mode connection's IP when invitation.address is empty.
///
/// # Errors
/// Returns an error on an unparsable address or SSRF prevention rejecting it.
pub fn resolve_session_addr(
    protocol_addr: SocketAddr,
    invitation: &SessionInvitation,
) -> Result<SocketAddr> {
    let addr = if invitation.address.is_empty() {
        let ip = protocol_addr.ip();
        SocketAddr::new(ip, invitation.port as u16)
    } else {
        let addr_str = String::from_utf8_lossy(&invitation.address);
        let ip: std::net::IpAddr = addr_str
            .parse()
            .map_err(|e| SyncthingError::network(format!("invalid session address: {}", e)))?;
        SocketAddr::new(ip, invitation.port as u16)
    };

    // SAFETY: SSRF protection — reject relays redirecting the client to unsafe addresses.
    if addr.ip().is_loopback() || addr.ip().is_multicast() || addr.ip().is_unspecified() {
        return Err(SyncthingError::network(format!(
            "SSRF prevention: relay attempted to redirect to unsafe address {}",
            addr
        )));
    }
    if let std::net::IpAddr::V4(v4) = addr.ip() {
        if v4.is_link_local() {
            return Err(SyncthingError::network(format!(
                "SSRF prevention: relay attempted to redirect to link-local address {}",
                addr
            )));
        }
    }

    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_relay_url_basic() {
        // 使用有效的 device ID（Base32-Luhn 格式）
        let valid_id = DeviceId::default().to_string();
        let (addr, id) =
            parse_relay_url(&format!("relay://192.168.1.1:22067/?id={}", valid_id)).unwrap();
        assert_eq!(addr.to_string(), "192.168.1.1:22067");
        assert_eq!(id, DeviceId::default()); // 成功解析
    }

    #[test]
    fn test_parse_relay_url_no_id() {
        let (addr, id) = parse_relay_url("relay://10.0.0.1:22067").unwrap();
        assert_eq!(addr.to_string(), "10.0.0.1:22067");
        assert_eq!(id, DeviceId::default());
    }

    #[test]
    fn test_resolve_session_addr_with_address() {
        let inv = SessionInvitation {
            from: vec![],
            key: vec![],
            address: b"203.0.113.5".to_vec(),
            port: 22067,
            server_socket: false,
        };
        let addr = resolve_session_addr("192.168.1.1:22067".parse().unwrap(), &inv).unwrap();
        assert_eq!(addr.to_string(), "203.0.113.5:22067");
    }

    #[test]
    fn test_resolve_session_addr_empty_fallback() {
        let inv = SessionInvitation {
            from: vec![],
            key: vec![],
            address: vec![],
            port: 443,
            server_socket: false,
        };
        let addr = resolve_session_addr("192.168.1.1:22067".parse().unwrap(), &inv).unwrap();
        assert_eq!(addr.to_string(), "192.168.1.1:443");
    }
}
