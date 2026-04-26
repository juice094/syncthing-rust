use std::sync::Arc;
use std::time::Duration;

use syncthing_core::{ConnectionState, ConnectionType, DeviceId, SyncthingError};
use syncthing_net::connection::{BepConnection, TcpBiStream};
use syncthing_net::handshaker::BepHandshaker;
use syncthing_net::tls::{accept_tls_stream, connect_tls_stream};
use tracing::{info, warn};

pub fn spawn_relay_listeners(
    relay_pool_urls: Vec<String>,
    config_relay_urls: Vec<String>,
    tls_config_arc: Arc<syncthing_net::SyncthingTlsConfig>,
    device_id: DeviceId,
    device_name: String,
    handle: syncthing_net::ConnectionManagerHandle,
) {
    let mut relay_listen_urls: std::collections::HashSet<String> = config_relay_urls.into_iter().collect();
    for url in relay_pool_urls {
        relay_listen_urls.insert(url);
    }
    let relay_listen_urls: Vec<String> = relay_listen_urls.into_iter().collect();
    for relay_url in relay_listen_urls {
        let relay_handle = handle.clone();
        let relay_tls = Arc::clone(&tls_config_arc);
        let relay_device_name = device_name.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            const MAX_BACKOFF: Duration = Duration::from_secs(300);
            const RESET_THRESHOLD: Duration = Duration::from_secs(60);
            loop {
                let start = std::time::Instant::now();
                match run_relay_listener(&relay_url, device_id, &relay_tls, &relay_handle, &relay_device_name).await {
                    Ok(()) => warn!("Relay listener for {} exited, reconnecting in {:?}", relay_url, backoff),
                    Err(e) => warn!("Relay listener for {} error: {}, reconnecting in {:?}", relay_url, e, backoff),
                }
                tokio::time::sleep(backoff).await;
                // Exponential backoff; reset if the listener ran successfully for a while
                if start.elapsed() > RESET_THRESHOLD {
                    backoff = Duration::from_secs(1);
                } else {
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        });
    }
}

/// Relay 永久 mode 监听循环
///
/// 连接到 relay 服务器，注册为可用节点，等待 SessionInvitation，
/// 收到后建立 session mode 连接并完成 BEP TLS + Hello，注册到 ConnectionManager。
async fn run_relay_listener(
    relay_url: &str,
    local_device_id: syncthing_core::DeviceId,
    tls_config: &Arc<syncthing_net::SyncthingTlsConfig>,
    handle: &syncthing_net::ConnectionManagerHandle,
    device_name: &str,
) -> syncthing_core::Result<()> {
    let (relay_addr, _) = syncthing_net::relay::dial::parse_relay_url(relay_url)?;
    let rustls_config = tls_config
        .relay_client_config()
        .map_err(|e| SyncthingError::Tls(format!("relay config: {}", e)))?;
    let rustls_config = std::sync::Arc::new(rustls_config);

    let mut client = syncthing_net::relay::RelayProtocolClient::connect(relay_addr, &rustls_config)
        .await?;
    client.join_relay().await?;
    info!("Relay listener joined {}", relay_addr);

    loop {
        let invitation = client.wait_invitation().await?;
        info!("Relay invitation received from {:?}", invitation.from);

        let session_addr = syncthing_net::relay::dial::resolve_session_addr(relay_addr, &invitation)?;
        let session_stream = syncthing_net::relay::join_session(session_addr, &invitation.key).await?;

        // BEP TLS + Hello + Connection 注册
        // accept_tls_stream / connect_tls_stream 返回不同类型，必须在分支内完成全部操作
        if invitation.server_socket {
            let (mut tls_stream, peer_device) = accept_tls_stream(session_stream, tls_config).await?;
            let _remote_hello = BepHandshaker::server_handshake(&mut tls_stream, device_name).await?;
            let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
            let conn = BepConnection::new(
                Box::new(TcpBiStream::Server(tls_stream)),
                ConnectionType::Incoming,
                event_tx,
            ).await?;
            conn.set_device_id(peer_device);
            conn.set_state(ConnectionState::ProtocolHandshakeComplete);
            handle.register_connection(peer_device, conn).await?;
            info!("Relay incoming (server) connection registered for {}", peer_device);
        } else {
            let (mut tls_stream, peer_device) = connect_tls_stream(session_stream, tls_config, Some(local_device_id)).await?;
            let _remote_hello = BepHandshaker::client_handshake(&mut tls_stream, device_name).await?;
            let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
            let conn = BepConnection::new(
                Box::new(TcpBiStream::Client(tls_stream)),
                ConnectionType::Incoming,
                event_tx,
            ).await?;
            conn.set_device_id(peer_device);
            conn.set_state(ConnectionState::ProtocolHandshakeComplete);
            handle.register_connection(peer_device, conn).await?;
            info!("Relay incoming (client) connection registered for {}", peer_device);
        }
    }
}
