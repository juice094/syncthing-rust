//! Relay 拨号辅助函数
//!
//! 将 Relay Protocol 与 BEP 连接层打通，提供 `connect_bep_via_relay`。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;
use tracing::{debug, info};

use syncthing_core::{ConnectionType, DeviceId, Result, SyncthingError};

use crate::connection::BepConnection;
use crate::handshaker::BepHandshaker;
use crate::relay::client::{join_session, RelayProtocolClient};
use crate::relay::protocol::SessionInvitation;
use crate::connection::TcpBiStream;
use crate::tls::{accept_tls_stream, connect_tls_stream, SyncthingTlsConfig};

/// 通过 relay 服务器建立与目标设备的 BEP 连接
///
/// 完整流程：
/// 1. Protocol Mode TLS → relay 服务器（ALPN = `bep-relay`）
/// 2. 发送 `ConnectRequest(target_device)`
/// 3. 接收 `SessionInvitation`
/// 4. Session Mode TCP → relay 的 session 端口
/// 5. 发送 `JoinSessionRequest(key)`
/// 6. 在 session stream 上启动 BEP TLS（client/server 由 `server_socket` 决定）
/// 7. BEP Hello 交换
/// 8. 创建 `BepConnection`
///
/// # 参数
/// - `relay_url`: relay 地址，如 `relay://relay.syncthing.net:22067/?id=ITZRNXE-...`
/// - `target_device`: 要连接的目标设备 ID
/// - `device_name`: 本机设备名称（用于 BEP Hello）
/// - `tls_config`: 本机 TLS 配置（含客户端证书）
pub async fn connect_bep_via_relay(
    relay_url: &str,
    target_device: DeviceId,
    device_name: &str,
    tls_config: &Arc<SyncthingTlsConfig>,
) -> Result<Arc<BepConnection>> {
    // 1. 解析 relay URL
    let (relay_addr, _relay_id) = parse_relay_url(relay_url)?;

    // 2. 建立 Protocol Mode TLS 连接（ALPN = bep-relay）
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

    // 3. 发送 ConnectRequest，获取 SessionInvitation
    let invitation = timeout(
        Duration::from_secs(60),
        protocol_client.request_session(target_device),
    )
    .await
    .map_err(|_| SyncthingError::timeout("relay session request timeout"))??;

    // 显式关闭 protocol 连接，避免与 listener 竞争 relay 槽位。
    // dialer 只需临时查询 invitation，不需要长期占用 protocol mode。
    drop(protocol_client);

    info!(
        "Relay session invitation: {}:{} (server_socket={})",
        String::from_utf8_lossy(&invitation.address),
        invitation.port,
        invitation.server_socket
    );

    // 4. 建立 Session Mode 连接
    let session_addr = resolve_session_addr(relay_addr, &invitation)?;
    let session_stream = timeout(
        Duration::from_secs(30),
        join_session(session_addr, &invitation.key),
    )
    .await
    .map_err(|_| SyncthingError::timeout("relay session join timeout"))??;

    debug!("Relay session mode connected to {}", session_addr);

    // 5. 在 session stream 上启动 BEP TLS 握手 + Hello + 创建连接
    let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();

    let conn = if invitation.server_socket {
        // 作为 TLS server
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

        debug!("Relay BEP TLS handshake completed with {} (server)", peer_device);

        let _remote_hello = BepHandshaker::server_handshake(&mut tls_stream, device_name).await?;

        BepConnection::new(
            Box::new(TcpBiStream::Server(tls_stream)),
            ConnectionType::Outgoing,
            event_tx,
        )
        .await?
    } else {
        // 作为 TLS client
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

        debug!("Relay BEP TLS handshake completed with {} (client)", peer_device);

        let _remote_hello = BepHandshaker::client_handshake(&mut tls_stream, device_name).await?;

        BepConnection::new(
            Box::new(TcpBiStream::Client(tls_stream)),
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

/// 解析 relay URL
///
/// 格式: `relay://host:port/?id=<device_id>`
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

/// 解析 session 地址
///
/// 如果 invitation.address 为空，回退到 protocol mode 连接的同一 IP。
pub fn resolve_session_addr(
    protocol_addr: SocketAddr,
    invitation: &SessionInvitation,
) -> Result<SocketAddr> {
    if invitation.address.is_empty() {
        let ip = protocol_addr.ip();
        Ok(SocketAddr::new(ip, invitation.port as u16))
    } else {
        let addr_str = String::from_utf8_lossy(&invitation.address);
        let ip: std::net::IpAddr = addr_str
            .parse()
            .map_err(|e| SyncthingError::network(format!("invalid session address: {}", e)))?;
        Ok(SocketAddr::new(ip, invitation.port as u16))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_relay_url_basic() {
        // 使用有效的 device ID（Base32-Luhn 格式）
        let valid_id = DeviceId::default().to_string();
        let (addr, id) = parse_relay_url(&format!("relay://192.168.1.1:22067/?id={}", valid_id)).unwrap();
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
