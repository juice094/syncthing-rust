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
use syncthing_net::{BepSession, BepSessionEvent, ConnectionManager, ConnectionManagerConfig, ConnectionManagerHandle, SyncthingTlsConfig};
use syncthing_net::protocol::MessageType;
use syncthing_sync::{database::FileSystemDatabase, SyncService, SyncManager, events::SyncEvent};

use crate::{ManagerBlockSource, load_config, save_config, CONFIG_FILE_NAME};

use syncthing_core::traits::ConfigStore;
use syncthing_api::config::JsonConfigStore;

use super::bep_handler::DaemonBepHandler;
use super::discovery_tasks::{GlobalDiscoveryShutdown, init_and_spawn_global_discovery, spawn_local_discovery};
use super::nat_tasks::{spawn_port_mapper, spawn_stun};
use super::relay_listener::spawn_relay_listeners;

/// Daemon 启动结果
pub struct DaemonStartup {
    pub future: std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>,
    pub connection_handle: ConnectionManagerHandle,
    pub sync_service: Arc<SyncService>,
    #[allow(dead_code)]
    pub session_handles: Arc<DashMap<DeviceId, JoinHandle<()>>>,
    pub device_id: DeviceId,
    #[allow(dead_code)]
    pub global_discovery_shutdown: Option<GlobalDiscoveryShutdown>,
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
        use rand::Rng;
        let api_key: String = (0..32)
            .map(|_| rand::thread_rng().gen_range(0..36))
            .map(|i| if i < 10 { (b'0' + i) as char } else { (b'a' + i - 10) as char })
            .collect();
        config.gui.api_key = api_key.clone();
        info!("Generated API key for REST API: {}... (masked)", &api_key[..4]);
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

    // ── NAT Traversal: STUN + PortMapper + Global Discovery (Phase 2/5) ──
    let local_port = actual_addr.port();
    let public_addrs = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));

    let cert_path = config_dir.join(syncthing_net::tls::CERT_FILE_NAME);
    let key_path = config_dir.join(syncthing_net::tls::KEY_FILE_NAME);
    let (global_discovery, global_discovery_shutdown) = init_and_spawn_global_discovery(
        device_id,
        &cert_path,
        &key_path,
        Arc::clone(&public_addrs),
        handle.clone(),
        Arc::clone(&sync_service),
        device_id,
    ).await;

    // PortMapper background task（含自动续约）
    spawn_port_mapper(actual_addr, local_port, Arc::clone(&public_addrs), global_discovery.clone());

    // STUN background task
    spawn_stun(local_port, Arc::clone(&public_addrs), global_discovery.clone());

    // ── Local Discovery (Phase 0: 恢复连接能力) ──
    spawn_local_discovery(device_id, actual_addr, Arc::clone(&public_addrs), handle.clone(), Arc::clone(&sync_service)).await;

    // 获取 relay pool（若启用），并进行分层健康检查
    let relay_pool_urls: Vec<String> = if config.options.relays_enabled {
        match syncthing_net::relay::fetch_relay_pool(None).await {
            Ok(urls) => {
                info!("Fetched {} relay(s) from pool, running staged health check...", urls.len());
                // Stage 1: lightweight TCP connect (all relays)
                let tcp_healthy = syncthing_net::relay::filter_healthy_relays(urls, 3).await;
                info!("{} relay(s) passed TCP health check", tcp_healthy.len());
                // Stage 2: deep TLS + JoinRelay on top 10, stop at 10 to increase overlap probability
                let to_check = tcp_healthy.into_iter().take(10).collect();
                let healthy = syncthing_net::relay::filter_healthy_relays_tls(to_check, 3, tls_config_arc.as_ref(), 10).await;
                info!("{} relay(s) passed TLS health check", healthy.len());
                healthy
            }
            Err(e) => {
                warn!("Failed to fetch relay pool: {}", e);
                Vec::new()
            }
        }
    } else {
        info!("Relay is disabled in config");
        Vec::new()
    };
    // 提取 TCP 直连地址和 Relay 地址
    let mut peers: Vec<(syncthing_core::DeviceId, Vec<SocketAddr>, Vec<String>)> = Vec::new();
    {
        let cfg = sync_service.get_config().await.unwrap_or_default();
        for d in cfg.devices.into_iter().filter(|d| d.id != device_id) {
            let tcp_addrs: Vec<SocketAddr> = d.addresses.iter().filter_map(|a| match a {
                syncthing_core::types::AddressType::Tcp(s) => s.parse().ok(),
                _ => None,
            }).collect();
            let mut relay_addrs: Vec<String> = d.addresses.iter().filter_map(|a| match a {
                syncthing_core::types::AddressType::Relay(s) => Some(s.clone()),
                _ => None,
            }).collect();
            // 若设备未配置 relay 地址但 relay pool 可用，使用所有 healthy relay 作为候选
            if relay_addrs.is_empty() {
                for url in &relay_pool_urls {
                    relay_addrs.push(url.clone());
                }
            }
            if !tcp_addrs.is_empty() || !relay_addrs.is_empty() {
                peers.push((d.id, tcp_addrs, relay_addrs));
            }
        }
    }

    // 并行拨号：direct + relay 统一竞速
    let handle_clone = handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        for (peer_id, addrs, relay_urls) in peers {
            info!("Auto-dialing peer {} at {:?} with {} relay candidates", peer_id, addrs, relay_urls.len());
            if let Err(e) = handle_clone.connect_to_with_relay(peer_id, addrs, relay_urls).await {
                warn!("Failed to auto-dial peer {}: {}", peer_id, e);
            }
        }
    });

    // ── Relay 被动监听（永久 mode）──
    let config_relay_urls: Vec<String> = {
        let cfg = sync_service.get_config().await.unwrap_or_default();
        cfg.devices.iter().filter_map(|d| {
            d.addresses.iter().find_map(|a| match a {
                syncthing_core::types::AddressType::Relay(url) => Some(url.clone()),
                _ => None,
            })
        }).collect()
    };
    spawn_relay_listeners(
        relay_pool_urls,
        config_relay_urls,
        Arc::clone(&tls_config_arc),
        device_id,
        config.device_name.clone(),
        handle.clone(),
    );

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

    // 配置热同步：监听 config.json 变更并通知 sync_service
    let config_path_for_watch = config_path.clone();
    let sync_service_for_watch = Arc::clone(&sync_service);
    tokio::spawn(async move {
        let store = JsonConfigStore::new(&config_path_for_watch);
        match store.watch().await {
            Ok(mut stream) => {
                while let Ok(()) = stream.next().await {
                    match store.load().await {
                        Ok(new_config) => {
                            if let Err(e) = sync_service_for_watch.update_config(new_config.clone()).await {
                                warn!("Failed to update sync service config from watch: {}", e);
                            } else {
                                info!("Config hot-reloaded from {:?}", config_path_for_watch);
                            }
                        }
                        Err(e) => warn!("Failed to load config for hot-reload: {}", e),
                    }
                }
            }
            Err(e) => warn!("Config watch setup failed: {}", e),
        }
    });

    Ok(DaemonStartup {
        future,
        connection_handle,
        sync_service,
        session_handles,
        device_id,
        global_discovery_shutdown,
    })
}
