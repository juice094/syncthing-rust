use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use syncthing_core::DeviceId;
use syncthing_net::ConnectionManagerHandle;
use syncthing_sync::{SyncService, SyncManager};

/// GlobalDiscovery 优雅退出 Drop guard
pub struct GlobalDiscoveryShutdown {
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

pub async fn init_and_spawn_global_discovery(
    device_id: DeviceId,
    cert_path: &Path,
    key_path: &Path,
    public_addrs: Arc<Mutex<Vec<String>>>,
    handle: ConnectionManagerHandle,
    sync_service: Arc<SyncService>,
    local_device_id: DeviceId,
) -> (Option<Arc<syncthing_net::GlobalDiscovery>>, Option<GlobalDiscoveryShutdown>) {
    let global_discovery = match syncthing_net::GlobalDiscovery::from_cert_files(device_id, cert_path, key_path, None).await {
        Ok(gd) => {
            info!("GlobalDiscovery initialized for {}", device_id);
            Some(Arc::new(gd))
        }
        Err(e) => {
            warn!("GlobalDiscovery initialization failed: {}", e);
            None
        }
    };

    let global_discovery_shutdown = global_discovery.as_ref().map(|gd| {
        GlobalDiscoveryShutdown::new(gd.shutdown_sender())
    });

    let global_discovery_query = global_discovery.clone();
    if let Some(gd) = global_discovery.clone() {
        let global_addrs = Arc::clone(&public_addrs);
        tokio::spawn(async move {
            gd.run(global_addrs).await;
        });
    }

    // Global Discovery periodic query task (Phase 5: feed peer addresses into ConnectionManager)
    if let Some(gd) = global_discovery_query {
        let query_handle = handle.clone();
        let query_devices: Vec<DeviceId> = sync_service.get_config().await
            .unwrap_or_default()
            .devices.into_iter()
            .filter(|d| d.id != local_device_id)
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

    (global_discovery, global_discovery_shutdown)
}

pub async fn spawn_local_discovery(
    device_id: DeviceId,
    actual_addr: SocketAddr,
    public_addrs: Arc<Mutex<Vec<String>>>,
    handle: ConnectionManagerHandle,
    sync_service: Arc<SyncService>,
) {
    let mut discovery_addrs = vec![format!("tcp://{}", actual_addr)];
    {
        let pa = public_addrs.lock().await;
        discovery_addrs.extend(pa.iter().cloned());
    }
    let (discovery_tx, mut discovery_rx) = tokio::sync::mpsc::channel::<syncthing_net::DiscoveryEvent>(32);
    let discovery_handle = handle.clone();
    let discovery_config = sync_service.get_config().await.unwrap_or_default();
    let known_device_ids: std::collections::HashSet<DeviceId> =
        discovery_config.devices.iter().map(|d| d.id).collect();

    tokio::spawn(async move {
        let discovery = syncthing_net::LocalDiscovery::new(device_id, discovery_addrs);
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
}
