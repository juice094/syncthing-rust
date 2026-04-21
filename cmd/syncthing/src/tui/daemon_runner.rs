use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::time::Duration;

use anyhow::{Context, Result};
use dashmap::DashMap;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use syncthing_core::types::Config;
use syncthing_core::DeviceId;
use syncthing_net::{BepSession, BepSessionEvent, BepSessionHandler, ConnectionManager, ConnectionManagerConfig, ConnectionManagerHandle, SyncthingTlsConfig};
use syncthing_net::protocol::MessageType;
use syncthing_sync::{database::FileSystemDatabase, SyncService, SyncModel, events::SyncEvent};

use crate::{ManagerBlockSource, load_config, save_config, CONFIG_FILE_NAME};

/// Daemon 启动结果
pub struct DaemonStartup {
    pub future: std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>,
    pub connection_handle: ConnectionManagerHandle,
    pub sync_service: Arc<SyncService>,
    #[allow(dead_code)]
    pub session_handles: Arc<DashMap<DeviceId, JoinHandle<()>>>,
    pub device_id: DeviceId,
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

    let mut config_modified = false;

    // 首次启动：将本地 device_id 持久化到配置文件
    if config.local_device_id != Some(device_id) {
        config.local_device_id = Some(device_id);
        config_modified = true;
        info!("Persisted local device_id to config");
    }

    // Port auto-migration removed: use CLI --listen flag if 22000 is occupied
    if config.gui.address == "0.0.0.0:8384" || config.gui.address == "127.0.0.1:8384" {
        warn!("Migrating gui.address from {} to 0.0.0.0:8385 to avoid conflict with local Go Syncthing", config.gui.address);
        config.gui.address = "0.0.0.0:8385".to_string();
        config_modified = true;
    }

    // 自动生成 API key（若为空）— 持久化操作
    if config.gui.api_key.is_empty() {
        let api_key: String = (0..32)
            .map(|_| rand::random::<u8>() % 36)
            .map(|i| if i < 10 { (b'0' + i) as char } else { (b'a' + i - 10) as char })
            .collect();
        config.gui.api_key = api_key.clone();
        info!("Generated API key for REST API: {}", api_key);
        config_modified = true;
    } else {
        info!("REST API enabled at {} with existing API key", config.gui.address);
    }

    if config_modified {
        if let Err(e) = save_config(&config_path, &config) {
            warn!("Failed to save config: {}", e);
        }
    }

    // Runtime overrides from CLI (Layer 3) — do not persist
    config.listen_addr = listen;
    config.device_name = device_name;


    let db_path = config_dir.join("db");
    let db = FileSystemDatabase::new(&db_path);
    let sync_service = Arc::new(SyncService::new(db).with_config(config.clone()).await);

    let manager_config = ConnectionManagerConfig {
        listen_addr,
        retry_config: syncthing_core::RetryConfig::default(),
        heartbeat_interval: Duration::from_secs(90),
        connection_timeout: Duration::from_secs(120),
        max_connections: 1000,
    };

    let tls_config_arc = Arc::new(tls_config);
    let identity = Arc::new(syncthing_net::identity::TlsIdentity::new(Arc::clone(&tls_config_arc)));
    let (manager, handle) =
        ConnectionManager::new(manager_config, identity, Arc::clone(&tls_config_arc));

    // Phase 2：注册 TransportRegistry，启用可插拔传输层
    let mut transport_registry = syncthing_net::transport::TransportRegistry::new();
    transport_registry.register(Arc::new(syncthing_net::transport::RawTcpTransport::new()));

    // Phase 3：注册 DERP relay 传输（用于 NAT 穿透失败时的中继回退）
    transport_registry.register(Arc::new(syncthing_net::derp::DerpTransport::new(device_id)));

    // 代理感知：若环境变量配置了代理，注册 ProxiedTransport
    if let Some(proxy_transport) = syncthing_net::transport::proxy::ProxiedTransport::from_env() {
        info!("Proxy detected: registering ProxiedTransport");
        transport_registry.register(Arc::new(proxy_transport));
    }

    manager.set_transport_registry(Arc::new(transport_registry));

    let pending_responses: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>> =
        Arc::new(DashMap::new());

    let block_source = Arc::new(ManagerBlockSource {
        manager: handle.clone(),
        next_id: AtomicI32::new(1),
        pending_responses: Arc::clone(&pending_responses),
    });
    sync_service.set_block_source(block_source).await;

    let session_handles: Arc<DashMap<DeviceId, JoinHandle<()>>> = Arc::new(DashMap::new());

    // 存储每个设备的共享文件夹列表（用于 IndexUpdate 过滤）
    let device_shared_folders: Arc<DashMap<DeviceId, Vec<String>>> = Arc::new(DashMap::new());

    let sync_service_clone = Arc::clone(&sync_service);
    let handle_clone = handle.clone();
    let pending_responses_clone = Arc::clone(&pending_responses);
    let session_handles_clone = Arc::clone(&session_handles);
    let device_shared_folders_clone = Arc::clone(&device_shared_folders);
    manager.on_connected(move |device_id| {
        syncthing_net::metrics::global().record_reconnect(device_id.to_string());
        info!("Device connected: {}", device_id);
        let sync_service = Arc::clone(&sync_service_clone);
        let handle = handle_clone.clone();
        let pending = Arc::clone(&pending_responses_clone);
        let sessions = Arc::clone(&session_handles_clone);
        let shared_folders_map = Arc::clone(&device_shared_folders_clone);
        tokio::spawn(async move {
            if let Err(e) = sync_service.connect_device(device_id).await {
                warn!("Failed to connect device {} to sync service: {}", device_id, e);
            }
            // Abort any existing session for this device before starting a new one
            if let Some((_, old_handle)) = sessions.remove(&device_id) {
                old_handle.abort();
            }
            let handle2 = tokio::spawn(async move {
                let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BepSessionEvent>();
                let event_device_id = device_id;
                let shared_folders_map = Arc::clone(&shared_folders_map);
                tokio::spawn(async move {
                    while let Some(event) = event_rx.recv().await {
                        match &event {
                            BepSessionEvent::ClusterConfigComplete { shared_folders, .. } => {
                                info!("[{}] ClusterConfig complete, shared folders: {:?}", event_device_id, shared_folders);
                                // 记录该设备的共享文件夹，用于后续 IndexUpdate 过滤
                                shared_folders_map.insert(event_device_id, shared_folders.clone());
                            }
                            BepSessionEvent::IndexSent { folder, file_count, .. } => {
                                info!("[{}] Index sent for {} ({} files)", event_device_id, folder, file_count);
                            }
                            BepSessionEvent::IndexReceived { folder, file_count, .. } => {
                                info!("[{}] Index received for {} ({} files)", event_device_id, folder, file_count);
                            }
                            BepSessionEvent::IndexUpdateReceived { folder, file_count, .. } => {
                                info!("[{}] IndexUpdate received for {} ({} files)", event_device_id, folder, file_count);
                            }
                            BepSessionEvent::BlockRequested { folder, name, offset, size, .. } => {
                                info!("[{}] Block requested: {}/{} offset={} size={}", event_device_id, folder, name, offset, size);
                            }
                            BepSessionEvent::HeartbeatTimeout { last_recv_age, .. } => {
                                warn!("[{}] Heartbeat timeout (idle {:?})", event_device_id, last_recv_age);
                            }
                            BepSessionEvent::PeerSyncState { folder, .. } => {
                                info!("[{}] Peer sync state changed for {}", event_device_id, folder);
                            }
                            BepSessionEvent::SessionEnded { reason, .. } => {
                                info!("[{}] Session ended: {}", event_device_id, reason);
                            }
                        }
                    }
                });

                let handler = DaemonBepHandler { sync_service: Arc::clone(&sync_service) };
                if let Some(conn) = handle.get_connection(&device_id) {
                    let session = BepSession::with_events(Arc::new(syncthing_core::DeviceIdentity::new(device_id)), conn, Arc::new(handler), pending, event_tx);
                    if let Err(e) = session.run().await {
                        warn!("BEP session for {} ended: {}", device_id, e);
                    }
                } else {
                    warn!("No connection for device {} to start BEP session", device_id);
                }
                let _ = handle.disconnect(&device_id, "bep session ended").await;
            });
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

    // 启动事件监听任务：当本地索引更新时，向共享该文件夹的已连接设备发送 IndexUpdate
    let event_sync_service = sync_service.clone();
    let event_handle = handle.clone();
    let device_shared_folders_clone = Arc::clone(&device_shared_folders);
    tokio::spawn(async move {
        let mut subscriber = event_sync_service.events().subscribe();
        while let Some(event) = subscriber.recv().await {
            if let SyncEvent::LocalIndexUpdated { folder, files } = event {
                if files.is_empty() {
                    continue;
                }
                // 防御性清空：确保 deleted 文件的 block list 为空（BEP 协议要求）
                let mut safe_files = files.clone();
                for file in &mut safe_files {
                    if file.is_deleted() {
                        file.blocks.clear();
                    }
                }
                let update = syncthing_core::types::IndexUpdate {
                    folder: folder.clone(),
                    files: safe_files,
                };
                let wire_update: bep_protocol::messages::IndexUpdate = update.into();
                match bep_protocol::messages::encode_message(&wire_update) {
                    Ok(payload) => {
                        for device_id in event_handle.connected_devices() {
                            // 只发送给共享该文件夹的设备
                            let should_send = match device_shared_folders_clone.get(&device_id) {
                                Some(entry) => entry.value().contains(&folder),
                                None => {
                                    // 尚未收到 ClusterConfig，保守起见不发送
                                    false
                                }
                            };
                            if !should_send {
                                continue;
                            }
                            if let Some(conn) = event_handle.get_connection(&device_id) {
                                if let Err(e) = conn.send_message(MessageType::IndexUpdate, payload.clone()).await {
                                    warn!("Failed to send IndexUpdate to {} for {}: {}", device_id, folder, e);
                                } else {
                                    info!("Sent IndexUpdate for {} to {} ({} files)", folder, device_id, files.len());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to encode IndexUpdate for {}: {}", folder, e);
                    }
                }
            }
        }
    });

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
        device_id,
    })
}

struct DaemonBepHandler {
    sync_service: Arc<SyncService>,
}

#[async_trait::async_trait]
impl BepSessionHandler for DaemonBepHandler {
    async fn generate_cluster_config(
        &self,
        device_id: DeviceId,
    ) -> syncthing_core::Result<bep_protocol::messages::ClusterConfig> {
        let config = self.sync_service.get_config().await.unwrap_or_default();
        let folders: Vec<bep_protocol::messages::WireFolder> = config
            .folders
            .iter()
            .filter(|f| f.devices.contains(&device_id))
            .map(|f| {
                let devices: Vec<bep_protocol::messages::WireDevice> = f
                    .devices
                    .iter()
                    .map(|d| bep_protocol::messages::WireDevice {
                        id: d.as_bytes().to_vec(),
                        name: String::new(),
                        addresses: vec![],
                        compression: bep_protocol::messages::Compression::Metadata as i32,
                        cert_name: String::new(),
                        max_sequence: 0,
                        introducer: false,
                        index_id: 0,
                        skip_introduction_removals: false,
                        encryption_password_token: Vec::new(),
                    })
                    .collect();
                bep_protocol::messages::WireFolder {
                    id: f.id.clone(),
                    label: vec![f.label.clone().unwrap_or_default()],
                    r#type: bep_protocol::messages::FolderType::SendReceive as i32,
                    stop_reason: bep_protocol::messages::FolderStopReason::Running as i32,
                    devices,
                }
            })
            .collect();

        Ok(bep_protocol::messages::ClusterConfig {
            folders,
            secondary: false,
        })
    }

    async fn generate_index(
        &self,
        folder_id: &str,
        _device_id: DeviceId,
    ) -> syncthing_core::Result<syncthing_core::types::Index> {
        let mut files = self.sync_service.generate_index_update(folder_id, 0).await.map_err(|e| {
            syncthing_core::SyncthingError::internal(format!("generate_index_update failed: {}", e))
        })?;
        // BEP protocol requires deleted files to have empty block lists
        for file in &mut files {
            if file.is_deleted() {
                file.blocks.clear();
            }
        }
        Ok(syncthing_core::types::Index {
            folder: folder_id.to_string(),
            files,
        })
    }

    async fn on_index(
        &self,
        device_id: DeviceId,
        index: syncthing_core::types::Index,
    ) -> syncthing_core::Result<()> {
        let folder = index.folder.clone();
        self.sync_service.handle_index(&folder, device_id, index).await.map_err(|e| {
            syncthing_core::SyncthingError::internal(format!("handle_index failed: {:?}", e))
        })?;
        Ok(())
    }

    async fn on_index_update(
        &self,
        device_id: DeviceId,
        update: syncthing_core::types::IndexUpdate,
    ) -> syncthing_core::Result<()> {
        let folder = update.folder.clone();
        self.sync_service
            .handle_index_update(&folder, device_id, update)
            .await
            .map_err(|e| {
                syncthing_core::SyncthingError::internal(format!("handle_index_update failed: {:?}", e))
            })?;
        Ok(())
    }

    async fn on_block_request(
        &self,
        _device_id: DeviceId,
        req: bep_protocol::messages::Request,
    ) -> std::result::Result<Vec<u8>, bep_protocol::messages::ErrorCode> {
        self.sync_service.handle_block_request(&req).await.map_err(|e| e.error_code())
    }
}


