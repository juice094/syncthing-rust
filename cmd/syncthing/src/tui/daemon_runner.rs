use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::time::Duration;

use anyhow::{Context, Result};
use dashmap::DashMap;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use syncthing_core::types::Config;
use syncthing_core::DeviceId;
use syncthing_net::{BepSession, BepSessionEvent, BepSessionHandler, ConnectionManager, ConnectionManagerConfig, ConnectionManagerHandle, SyncthingTlsConfig};
use syncthing_net::protocol::MessageType;
use syncthing_sync::{database::FileSystemDatabase, SyncService, SyncModel, events::SyncEvent};

use crate::{ManagerBlockSource, load_config, save_config, CONFIG_FILE_NAME};

use syncthing_core::traits::ConfigStore;
use syncthing_api::config::JsonConfigStore;

/// GlobalDiscovery 优雅退出 Drop guard
pub(crate) struct GlobalDiscoveryShutdown {
    tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl GlobalDiscoveryShutdown {
    fn new(tx: tokio::sync::broadcast::Sender<()>) -> Self {
        Self { tx: Some(tx) }
    }
}

impl Drop for GlobalDiscoveryShutdown {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(());
        }
    }
}

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

    // ── NAT Traversal: STUN + PortMapper + Global Discovery (Phase 2/5) ──
    let local_port = actual_addr.port();
    let public_addrs = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));

    // 预先创建 GlobalDiscovery，成功后共享给 STUN/PortMapper 用于触发 re-announce
    let cert_path = config_dir.join(syncthing_net::tls::CERT_FILE_NAME);
    let key_path = config_dir.join(syncthing_net::tls::KEY_FILE_NAME);
    let global_discovery = match syncthing_net::GlobalDiscovery::from_cert_files(device_id, &cert_path, &key_path, None).await {
        Ok(gd) => {
            info!("GlobalDiscovery initialized for {}", device_id);
            Some(Arc::new(gd))
        }
        Err(e) => {
            warn!("GlobalDiscovery initialization failed: {}", e);
            None
        }
    };

    // PortMapper background task（含自动续约）
    let pm_public_addrs = Arc::clone(&public_addrs);
    let pm_gd = global_discovery.clone();
    tokio::spawn(async move {
        let mut port_mapper = syncthing_net::PortMapper::new()
            .with_local_addr(actual_addr);
        match port_mapper.allocate_port(local_port).await {
            Ok(mut mapping) => {
                let mut current_ext = mapping.external_addr();
                let ext_url = format!("tcp://{}", current_ext);
                info!("PortMapper success: {} -> {}", actual_addr, ext_url);
                pm_public_addrs.lock().await.push(ext_url);
                if let Some(ref gd) = pm_gd {
                    gd.trigger_reannounce();
                }

                // 自动续约循环（每 lifetime/2 续约一次）
                loop {
                    let now = std::time::Instant::now();
                    let renew_after = mapping.renew_after();
                    if renew_after <= now {
                        warn!("PortMapper mapping expired before renewal");
                        break;
                    }
                    let sleep_duration = renew_after.duration_since(now);
                    tokio::time::sleep(sleep_duration).await;

                    match port_mapper.allocate_port(local_port).await {
                        Ok(new_mapping) => {
                            let new_ext = new_mapping.external_addr();
                            info!("PortMapper renewed: {} -> {}", actual_addr, new_ext);

                            // 更新 public_addrs：替换旧的外部地址
                            {
                                let mut addrs = pm_public_addrs.lock().await;
                                let old_url = format!("tcp://{}", current_ext);
                                if let Some(pos) = addrs.iter().position(|a| a == &old_url) {
                                    addrs.remove(pos);
                                }
                                addrs.push(format!("tcp://{}", new_ext));
                            }

                            if let Some(ref gd) = pm_gd {
                                gd.trigger_reannounce();
                            }

                            current_ext = new_ext;
                            mapping = new_mapping;
                        }
                        Err(e) => {
                            warn!("PortMapper renewal failed: {}", e);
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                warn!("PortMapper failed (expected if router has no UPnP/NAT-PMP): {}", e);
            }
        }
    });

    // STUN background task
    let stun_public_addrs = Arc::clone(&public_addrs);
    let stun_gd = global_discovery.clone();
    tokio::spawn(async move {
        let stun = syncthing_net::StunClient::new()
            .with_local_port(local_port);
        match stun.get_public_address().await {
            Ok(pub_addr) => {
                let pub_url = format!("tcp://{}", pub_addr);
                info!("STUN public address: {}", pub_addr);
                stun_public_addrs.lock().await.push(pub_url);
                if let Some(gd) = stun_gd {
                    gd.trigger_reannounce();
                }
            }
            Err(e) => {
                warn!("STUN detection failed (expected behind symmetric NAT/firewall): {}", e);
            }
        }
    });

    // 在移动 global_discovery 之前捕获 shutdown sender
    let global_discovery_shutdown = global_discovery.as_ref().map(|gd| {
        GlobalDiscoveryShutdown::new(gd.shutdown_sender())
    });

    // Global Discovery background task (announce)
    let global_discovery_query = global_discovery.clone();
    if let Some(gd) = global_discovery {
        let global_addrs = Arc::clone(&public_addrs);
        tokio::spawn(async move {
            gd.run(global_addrs).await;
        });
    }

    // Global Discovery periodic query task (Phase 5: feed peer addresses into ConnectionManager)
    if let Some(gd) = global_discovery_query {
        let query_handle = handle.clone();
        let query_devices: Vec<syncthing_core::DeviceId> = sync_service.get_config().await
            .unwrap_or_default()
            .devices.into_iter()
            .filter(|d| d.id != device_id)
            .map(|d| d.id)
            .collect();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                for peer_id in &query_devices {
                    match gd.query(*peer_id).await {
                        Ok(urls) => {
                            let tcp_addrs: Vec<SocketAddr> = urls.iter().filter_map(|u| {
                                u.strip_prefix("tcp://").and_then(|s| s.parse().ok())
                            }).collect();
                            let relay_urls: Vec<String> = urls.iter().filter_map(|u| {
                                if u.starts_with("relay://") { Some(u.clone()) } else { None }
                            }).collect();
                            if !tcp_addrs.is_empty() || !relay_urls.is_empty() {
                                info!("Global discovery: updating {} with {} tcp + {} relay addr(s)", peer_id, tcp_addrs.len(), relay_urls.len());
                                query_handle.update_addresses(*peer_id, tcp_addrs, relay_urls);
                            }
                        }
                        Err(e) => {
                            debug!("Global discovery query for {} failed: {}", peer_id, e);
                        }
                    }
                }
            }
        });
    }

    // ── Local Discovery (Phase 0: 恢复连接能力) ──
    let discovery_device_id = device_id;
    let mut discovery_addrs = vec![format!("tcp://{}", actual_addr)];
    // Append any already-known public addresses
    {
        let pa = public_addrs.lock().await;
        discovery_addrs.extend(pa.iter().cloned());
    }
    let (discovery_tx, mut discovery_rx) = tokio::sync::mpsc::channel::<syncthing_net::DiscoveryEvent>(32);
    let discovery_handle = handle.clone();
    let discovery_config = sync_service.get_config().await.unwrap_or_default();
    let known_device_ids: std::collections::HashSet<syncthing_core::DeviceId> =
        discovery_config.devices.iter().map(|d| d.id).collect();

    tokio::spawn(async move {
        let discovery = syncthing_net::LocalDiscovery::new(discovery_device_id, discovery_addrs);
        if let Err(e) = discovery.run(discovery_tx).await {
            warn!("Local discovery error: {}", e);
        }
    });

    tokio::spawn(async move {
        while let Some(event) = discovery_rx.recv().await {
            if let syncthing_net::DiscoveryEvent::DeviceDiscovered { device_id, addresses, .. } = event {
                if known_device_ids.contains(&device_id) {
                    let addrs: Vec<SocketAddr> = addresses.iter()
                        .filter_map(|a| a.parse().ok())
                        .collect();
                    if !addrs.is_empty() {
                        info!("Local discovery: discovered {} at {:?}, updating address pool", device_id, addrs);
                        discovery_handle.update_addresses(device_id, addrs.clone(), vec![]);
                        if discovery_handle.get_connection(&device_id).is_none() {
                            if let Err(e) = discovery_handle.connect_to(device_id, addrs).await {
                                warn!("Failed to auto-dial discovered device {}: {}", device_id, e);
                            }
                        }
                    }
                }
            }
        }
    });

    // 获取 relay pool（若启用），并进行分层健康检查
    let relay_pool_urls: Vec<String> = if config.options.relays_enabled {
        match syncthing_net::relay::fetch_relay_pool(None).await {
            Ok(urls) => {
                info!("Fetched {} relay(s) from pool, running staged health check...", urls.len());
                // Stage 1: lightweight TCP connect (all relays)
                let tcp_healthy = syncthing_net::relay::filter_healthy_relays(urls, 3).await;
                info!("{} relay(s) passed TCP health check", tcp_healthy.len());
                // Stage 2: deep TLS + JoinRelay on top 10, stop at 3 to reduce startup latency
                let to_check = tcp_healthy.into_iter().take(10).collect();
                let healthy = syncthing_net::relay::filter_healthy_relays_tls(to_check, 3, tls_config_arc.as_ref(), 3).await;
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
    let first_pool_relay = relay_pool_urls.first().cloned();

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
            // 若设备未配置 relay 地址但 relay pool 可用，使用 pool 中的第一个作为 fallback
            if relay_addrs.is_empty() {
                if let Some(ref url) = first_pool_relay {
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
    // 收集配置中的 relay 地址（去重），若不足 3 个用 pool 补齐，分别 spawn 监听任务
    let mut relay_listen_urls: std::collections::HashSet<String> = {
        let cfg = sync_service.get_config().await.unwrap_or_default();
        cfg.devices.iter().filter_map(|d| {
            d.addresses.iter().find_map(|a| match a {
                syncthing_core::types::AddressType::Relay(url) => Some(url.clone()),
                _ => None,
            })
        }).collect()
    };
    for url in relay_pool_urls {
        relay_listen_urls.insert(url);
    }
    let relay_listen_urls: Vec<String> = relay_listen_urls.into_iter().take(3).collect();
    for relay_url in relay_listen_urls {
        let relay_handle = handle.clone();
        let relay_tls = Arc::clone(&tls_config_arc);
        let relay_device_name = config.device_name.clone();
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
    use syncthing_core::{ConnectionState, ConnectionType, SyncthingError};
    use syncthing_net::connection::{BepConnection, TcpBiStream};
    use syncthing_net::handshaker::BepHandshaker;
    use syncthing_net::tls::{accept_tls_stream, connect_tls_stream};
    use tracing::info;

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
        let local_id = config.local_device_id.unwrap_or_default();
        let folders: Vec<bep_protocol::messages::WireFolder> = config
            .folders
            .iter()
            .filter(|f| f.devices.contains(&device_id))
            .map(|f| {
                let mut devices: Vec<bep_protocol::messages::WireDevice> = f
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
                // BEP 协议要求 ClusterConfig 的 devices 列表必须包含本地设备
                if !f.devices.contains(&local_id) {
                    devices.push(bep_protocol::messages::WireDevice {
                        id: local_id.as_bytes().to_vec(),
                        name: String::new(),
                        addresses: vec![],
                        compression: bep_protocol::messages::Compression::Metadata as i32,
                        cert_name: String::new(),
                        max_sequence: 0,
                        introducer: false,
                        index_id: 0,
                        skip_introduction_removals: false,
                        encryption_password_token: Vec::new(),
                    });
                }
                bep_protocol::messages::WireFolder {
                    id: f.id.clone(),
                    label: f.label.clone().unwrap_or_default(),
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


