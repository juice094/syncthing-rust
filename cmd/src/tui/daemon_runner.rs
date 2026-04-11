use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dashmap::DashMap;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use syncthing_core::types::Config;
use syncthing_core::{ConnectionState, DeviceId};
use syncthing_net::{ConnectionManager, ConnectionManagerConfig, ConnectionManagerHandle, SyncthingTlsConfig};
use syncthing_net::protocol::MessageType;
use syncthing_net::metrics;
use syncthing_sync::{database::MemoryDatabase, SyncService, SyncModel, BlockSource};

use crate::{ManagerBlockSource, load_config, save_config, CONFIG_FILE_NAME};

/// Daemon 启动结果
pub struct DaemonStartup {
    pub future: std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>,
    pub connection_handle: ConnectionManagerHandle,
    pub sync_service: Arc<SyncService>,
    pub session_handles: Arc<DashMap<DeviceId, JoinHandle<()>>>,
}

/// 启动 daemon，返回 future 和句柄
pub async fn start_daemon(
    config_dir: PathBuf,
    listen: String,
    device_name: String,
) -> Result<DaemonStartup> {
    info!("Starting Syncthing Rust daemon from TUI...");

    let tls_config = SyncthingTlsConfig::load_or_generate(&config_dir)
        .await
        .context("failed to load or generate certificate")?;

    let device_id = tls_config.device_id();
    info!("Device ID: {}", device_id);
    info!("Device Name: {}", device_name);

    let listen_addr: SocketAddr = listen.parse().context("invalid listen address")?;

    let config_path = config_dir.join(CONFIG_FILE_NAME);
    let mut config = if config_path.exists() {
        load_config(&config_path)
            .unwrap_or_else(|e| {
                warn!("Failed to load config: {}. Using default.", e);
                Config::new()
            })
    } else {
        Config::new()
    };
    config.local_device_id = Some(device_id);

    // 互操作测试自动配置（仅开发阶段）
    if config.devices.is_empty() && config.folders.is_empty() {
        let go_cert_path = std::path::PathBuf::from(r"C:\Users\22414\AppData\Local\Temp\syncthing_test_go\cert.pem");
        let go_key_path = std::path::PathBuf::from(r"C:\Users\22414\AppData\Local\Temp\syncthing_test_go\key.pem");
        if go_cert_path.exists() && go_key_path.exists() {
            let cert = tokio::fs::read(&go_cert_path).await.unwrap_or_default();
            let key = tokio::fs::read(&go_key_path).await.unwrap_or_default();
            if let Ok(cfg) = SyncthingTlsConfig::from_pem(&cert, &key) {
                let go_device_id = cfg.device_id();
                config.devices.push(syncthing_core::types::Device {
                    id: go_device_id,
                    name: Some("go-syncthing".to_string()),
                    addresses: vec![syncthing_core::types::AddressType::Tcp("127.0.0.1:22001".to_string())],
                    paused: false,
                    introducer: false,
                });
                let test_folder_path = std::env::temp_dir().join("syncthing_test_rust_folder");
                std::fs::create_dir_all(&test_folder_path).ok();
                let mut test_folder = syncthing_core::types::Folder::new("test-folder", test_folder_path.to_string_lossy());
                test_folder.devices.push(device_id);
                test_folder.devices.push(go_device_id);
                config.folders.push(test_folder);
                if let Err(e) = save_config(&config_path, &config) {
                    warn!("Failed to save auto-generated interop config: {}", e);
                }
            }
        }
    }

    if let Err(e) = save_config(&config_path, &config) {
        warn!("Failed to save config: {}", e);
    }

    let db = MemoryDatabase::new();
    let sync_service = Arc::new(SyncService::new(db).with_config(config.clone()).await);

    let manager_config = ConnectionManagerConfig {
        listen_addr,
        retry_config: syncthing_core::RetryConfig::default(),
        heartbeat_interval: Duration::from_secs(90),
        connection_timeout: Duration::from_secs(120),
        max_connections: 1000,
    };

    let tls_config_arc = Arc::new(tls_config);
    let (manager, handle) =
        ConnectionManager::new(manager_config, device_id, Arc::clone(&tls_config_arc));

    let pending_responses: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>> =
        Arc::new(DashMap::new());

    let block_source = Arc::new(ManagerBlockSource {
        manager: handle.clone(),
        next_id: AtomicI32::new(1),
        pending_responses: Arc::clone(&pending_responses),
    });
    sync_service.set_block_source(block_source).await;

    let session_handles: Arc<DashMap<DeviceId, JoinHandle<()>>> = Arc::new(DashMap::new());

    let sync_service_clone = Arc::clone(&sync_service);
    let handle_clone = handle.clone();
    let pending_responses_clone = Arc::clone(&pending_responses);
    let session_handles_clone = Arc::clone(&session_handles);
    manager.on_connected(move |device_id| {
        metrics::global().record_reconnect(device_id.to_string());
        info!("Device connected: {}", device_id);
        let sync_service = Arc::clone(&sync_service_clone);
        let handle = handle_clone.clone();
        let pending = Arc::clone(&pending_responses_clone);
        let sessions = Arc::clone(&session_handles_clone);
        tokio::spawn(async move {
            if let Err(e) = sync_service.connect_device(device_id).await {
                warn!("Failed to connect device {} to sync service: {}", device_id, e);
            }
            let handle2 = tokio::spawn(run_bep_session(device_id, handle, sync_service, pending));
            sessions.insert(device_id, handle2);
        });
    });

    let sync_service_clone = Arc::clone(&sync_service);
    let session_handles_clone = Arc::clone(&session_handles);
    manager.on_disconnected(move |device_id, reason| {
        warn!("Device disconnected: {} - {}", device_id, reason);
        let sync_service = Arc::clone(&sync_service_clone);
        let sessions = Arc::clone(&session_handles_clone);
        tokio::spawn(async move {
            if let Err(e) = sync_service.disconnect_device(device_id).await {
                warn!("Failed to disconnect device {} from sync service: {}", device_id, e);
            }
            if let Some((_, handle)) = sessions.remove(&device_id) {
                handle.abort();
            }
        });
    });

    sync_service
        .start()
        .await
        .context("failed to start sync service")?;
    info!("Sync service started");

    let actual_addr = manager
        .start()
        .await
        .context("failed to start connection manager")?;
    info!("Listening on: {}", actual_addr);

    let peers: Vec<(syncthing_core::DeviceId, Vec<SocketAddr>)> = {
        let cfg = sync_service.get_config().await.unwrap_or_default();
        cfg.devices
            .into_iter()
            .filter(|d| d.id != device_id)
            .filter_map(|d: syncthing_core::types::Device| {
                let addrs: Vec<SocketAddr> = d.addresses.iter().filter_map(|a: &syncthing_core::types::AddressType| a.as_str().parse().ok()).collect();
                if addrs.is_empty() { None } else { Some((d.id, addrs)) }
            })
            .collect()
    };
    let handle_clone = handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        for (peer_id, addrs) in peers {
            info!("Auto-dialing peer {} at {:?}", peer_id, addrs);
            if let Err(e) = handle_clone.connect_to(peer_id, addrs).await {
                warn!("Failed to auto-dial peer {}: {}", peer_id, e);
            }
        }
    });

    let connection_handle = handle.clone();
    let session_handles_clone = Arc::clone(&session_handles);
    let future: std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>> = Box::pin(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            // Clean up finished session handles
            let finished: Vec<DeviceId> = session_handles_clone
                .iter()
                .filter(|entry| entry.value().is_finished())
                .map(|entry| *entry.key())
                .collect();
            for d in finished {
                session_handles_clone.remove(&d);
            }
        }
    });

    Ok(DaemonStartup {
        future,
        connection_handle,
        sync_service,
        session_handles,
    })
}

async fn run_bep_session(
    device_id: DeviceId,
    handle: ConnectionManagerHandle,
    sync_service: Arc<SyncService>,
    pending_responses: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>>,
) {
    let conn = match handle.get_connection(&device_id) {
        Some(c) => c,
        None => {
            warn!("No connection for device {} to start BEP session", device_id);
            return;
        }
    };

    // 1. Send ClusterConfig
    let config = sync_service.get_config().await.unwrap_or_default();
    let local_device_id = config.local_device_id.unwrap_or(device_id);
    let folders: Vec<bep_protocol::messages::WireFolder> = config.folders
        .iter()
        .filter(|f| f.devices.contains(&device_id))
        .map(|f| {
            let devices: Vec<bep_protocol::messages::WireDevice> = f.devices.iter().map(|d| {
                bep_protocol::messages::WireDevice {
                    id: d.as_bytes().to_vec(),
                    name: String::new(),
                    addresses: vec![],
                    compression: bep_protocol::messages::Compression::Metadata as i32,
                    cert_name: String::new(),
                    max_sequence: 0,
                }
            }).collect();
            bep_protocol::messages::WireFolder {
                id: f.id.clone(),
                label: vec![f.label.clone().unwrap_or_default()],
                r#type: bep_protocol::messages::FolderType::SendReceive as i32,
                stop_reason: bep_protocol::messages::FolderStopReason::Running as i32,
                devices,
            }
        })
        .collect();

    let cc = bep_protocol::messages::ClusterConfig { folders };
    if let Err(e) = conn.send_cluster_config(&cc).await {
        warn!("Failed to send ClusterConfig to {}: {}", device_id, e);
        return;
    }
    info!("Sent ClusterConfig to {} ({} folders)", device_id, cc.folders.len());

    // 2. Wait for remote ClusterConfig
    let mut remote_cc_received = false;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), conn.recv_message()).await {
            Ok(Ok((msg_type, payload))) => {
                match msg_type {
                    MessageType::ClusterConfig => {
                        match bep_protocol::messages::decode_message::<bep_protocol::messages::ClusterConfig>(&payload) {
                            Ok(remote_cc) => {
                                info!("Received ClusterConfig from {} ({} folders)", device_id, remote_cc.folders.len());
                                remote_cc_received = true;
                                conn.set_state(ConnectionState::ClusterConfigComplete);
                                break;
                            }
                            Err(e) => {
                                warn!("Failed to decode ClusterConfig from {}: {}", device_id, e);
                            }
                        }
                    }
                    MessageType::Ping => {
                        if let Err(e) = conn.send_ping().await {
                            warn!("Failed to send ping reply to {}: {}", device_id, e);
                        }
                    }
                    _ => {
                        debug!("Ignoring message {:?} before ClusterConfig complete", msg_type);
                    }
                }
            }
            Ok(Err(e)) => {
                warn!("Connection error with {}: {}", device_id, e);
                return;
            }
            Err(_) => {
                warn!("Timeout waiting for ClusterConfig from {}", device_id);
                return;
            }
        }
    }

    if !remote_cc_received {
        return;
    }

    // 3. Send Index for each shared folder
    let shared_folder_ids: Vec<String> = cc.folders.into_iter().map(|f| f.id).collect();
    for folder_id in &shared_folder_ids {
        match sync_service.generate_index_update(folder_id, 0).await {
            Ok(files) => {
                let index = syncthing_core::types::Index {
                    folder: folder_id.clone(),
                    files,
                };
                if let Err(e) = conn.send_index(&index).await {
                    warn!("Failed to send Index for {} to {}: {}", folder_id, device_id, e);
                } else {
                    info!("Sent Index for {} to {} ({} files)", folder_id, device_id, index.files.len());
                }
            }
            Err(e) => {
                warn!("Failed to generate index for {}: {}", folder_id, e);
            }
        }
    }

    // 4. Steady-state message loop
    info!("Entering steady-state BEP loop for {}", device_id);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(90));
    let mut last_recv = Instant::now();
    loop {
        tokio::select! {
            result = conn.recv_message() => {
                let latency = last_recv.elapsed();
                match result {
                    Ok((msg_type, payload)) => {
                        metrics::global().record_bep_message_recv(
                            device_id.to_string(),
                            &format!("{:?}", msg_type),
                            latency,
                            payload.len() as u64,
                        );
                        last_recv = Instant::now();
                        match msg_type {
                            MessageType::Ping => {
                                if let Err(e) = conn.send_ping().await {
                                    warn!("Failed to send ping reply to {}: {}", device_id, e);
                                    break;
                                }
                            }
                            MessageType::Index => {
                                match bep_protocol::messages::decode_message::<bep_protocol::messages::Index>(&payload) {
                                    Ok(wire_index) => {
                                        let index: syncthing_core::types::Index = wire_index.into();
                                        let folder = index.folder.clone();
                                        if let Err(e) = sync_service.handle_index(&folder, device_id, index).await {
                                            warn!("Failed to handle Index from {}: {}", device_id, e);
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode Index from {}: {}", device_id, e),
                                }
                            }
                            MessageType::IndexUpdate => {
                                match bep_protocol::messages::decode_message::<bep_protocol::messages::IndexUpdate>(&payload) {
                                    Ok(wire_update) => {
                                        let update: syncthing_core::types::IndexUpdate = wire_update.into();
                                        let folder = update.folder.clone();
                                        if let Err(e) = sync_service.handle_index_update(&folder, device_id, update).await {
                                            warn!("Failed to handle IndexUpdate from {}: {}", device_id, e);
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode IndexUpdate from {}: {}", device_id, e),
                                }
                            }
                            MessageType::Request => {
                                match bep_protocol::messages::decode_message::<bep_protocol::messages::Request>(&payload) {
                                    Ok(req) => {
                                        if let Err(e) = handle_request(&conn, &sync_service, req).await {
                                            warn!("Failed to handle Request from {}: {}", device_id, e);
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode Request from {}: {}", device_id, e),
                                }
                            }
                            MessageType::Response => {
                                match bep_protocol::messages::decode_message::<bep_protocol::messages::Response>(&payload) {
                                    Ok(resp) => {
                                        if let Some((_, tx)) = pending_responses.remove(&resp.id) {
                                            let _ = tx.send(resp);
                                        } else {
                                            warn!("Received unmatched Response id={} from {}", resp.id, device_id);
                                        }
                                    }
                                    Err(e) => warn!("Failed to decode Response from {}: {}", device_id, e),
                                }
                            }
                            MessageType::Close => {
                                info!("Received Close from {}", device_id);
                                break;
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        warn!("BEP session loop error for {}: {}", device_id, e);
                        break;
                    }
                }
            }
            _ = heartbeat.tick() => {
                if let Err(e) = conn.send_ping().await {
                    warn!("Failed to send ping to {}: {}", device_id, e);
                    break;
                }
            }
        }
    }

    // Disconnect cleanly
    let _ = handle.disconnect(&device_id, "bep session ended").await;
}

async fn handle_request(
    conn: &syncthing_net::connection::BepConnection,
    sync_service: &SyncService,
    req: bep_protocol::messages::Request,
) -> anyhow::Result<()> {
    let config = sync_service.get_config().await.unwrap_or_default();
    let folder = config.folders.into_iter().find(|f| f.id == req.folder);
    let folder_path = match folder {
        Some(f) => f.path,
        None => {
            let resp = bep_protocol::messages::Response {
                id: req.id,
                data: vec![],
                error: "folder not found".to_string(),
            };
            let payload = bep_protocol::messages::encode_message(&resp)?;
            conn.send_message(MessageType::Response, payload).await?;
            return Ok(());
        }
    };

    let file_path = std::path::PathBuf::from(&folder_path).join(&req.name);
    let mut file = match tokio::fs::File::open(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            let resp = bep_protocol::messages::Response {
                id: req.id,
                data: vec![],
                error: format!("open failed: {}", e),
            };
            let payload = bep_protocol::messages::encode_message(&resp)?;
            conn.send_message(MessageType::Response, payload).await?;
            return Ok(());
        }
    };

    if let Err(e) = file.seek(std::io::SeekFrom::Start(req.offset as u64)).await {
        let resp = bep_protocol::messages::Response {
            id: req.id,
            data: vec![],
            error: format!("seek failed: {}", e),
        };
        let payload = bep_protocol::messages::encode_message(&resp)?;
        conn.send_message(MessageType::Response, payload).await?;
        return Ok(());
    }

    let size = req.size.max(0) as usize;
    let mut buf = vec![0u8; size];
    let n = match file.read(&mut buf).await {
        Ok(n) => n,
        Err(e) => {
            let resp = bep_protocol::messages::Response {
                id: req.id,
                data: vec![],
                error: format!("read failed: {}", e),
            };
            let payload = bep_protocol::messages::encode_message(&resp)?;
            conn.send_message(MessageType::Response, payload).await?;
            return Ok(());
        }
    };
    buf.truncate(n);

    let resp = bep_protocol::messages::Response {
        id: req.id,
        data: buf,
        error: String::new(),
    };
    let payload = bep_protocol::messages::encode_message(&resp)?;
    conn.send_message(MessageType::Response, payload).await?;
    Ok(())
}
