//! Minimal headless peer for cross-device sync tests.
//!
//! Usage:
//!   cargo run --example peer_node -- <config-dir> [listen-addr] [folder-id] [folder-path] [peer-device-id...]
//!
//! Defaults: listen 0.0.0.0:22002, folder "obsidian-sync", folder-path <config-dir>/sync

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use syncthing_core::types::{Config, Device, Folder, GuiConfig, Options};
use syncthing_core::DeviceId;
use syncthing_net::{
    identity::TlsIdentity, ConnectionManager, ConnectionManagerConfig, SyncthingTlsConfig,
};
use syncthing_sync::{database::MemoryDatabase, SyncManager, SyncService};

use syncthing_test_utils::bep_bridge::{install_bep_bridge, TestBlockSource};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("syncthing=info".parse().unwrap()),
        )
        .init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: peer_node <config-dir> [listen-addr] [folder-id] [folder-path] [peer-device-id...]");
        std::process::exit(1);
    }

    let config_dir = PathBuf::from(&args[1]);
    let listen_addr: SocketAddr = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "0.0.0.0:22002".to_string())
        .parse()?;
    let folder_id = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "obsidian-sync".to_string());
    let folder_path = args
        .get(4)
        .cloned()
        .map(PathBuf::from)
        .unwrap_or_else(|| config_dir.join("sync"));
    let peer_ids: Vec<DeviceId> = args
        .iter()
        .skip(5)
        .map(|s| s.parse())
        .collect::<Result<Vec<_>, _>>()?;

    tokio::fs::create_dir_all(&config_dir).await?;
    tokio::fs::create_dir_all(&folder_path).await?;

    let tls_config = SyncthingTlsConfig::load_or_generate(&config_dir).await?;
    let device_id = tls_config.device_id();
    let tls_config_arc = Arc::new(tls_config);

    let mut config = Config::new();
    config.device_name = "desktop-peer".to_string();
    config.listen_addr = listen_addr.to_string();
    config.gui = GuiConfig {
        enabled: false,
        address: "127.0.0.1:0".to_string(),
        api_key: "e2e-test-key".to_string(),
        ro_api_key: String::new(),
    };
    config.options = Options {
        relays_enabled: false,
        ..Default::default()
    };

    for peer_id in &peer_ids {
        config.devices.push(Device {
            id: *peer_id,
            name: Some(peer_id.short_id().to_string()),
            addresses: vec![],
            paused: false,
            introducer: false,
        });
    }

    let config_path = config_dir.join("config.json");
    tokio::fs::write(&config_path, serde_json::to_string_pretty(&config)?).await?;

    let db = MemoryDatabase::new();
    let sync_service = Arc::new(SyncService::new(db).with_config(config).await);
    sync_service.start().await?;

    let manager_config = ConnectionManagerConfig {
        listen_addr,
        ..Default::default()
    };
    let identity = Arc::new(TlsIdentity::new(Arc::clone(&tls_config_arc)));
    let (manager, connection_handle) =
        ConnectionManager::new(manager_config, identity, tls_config_arc);

    let mut registry = syncthing_net::transport::TransportRegistry::new();
    registry.register(Arc::new(syncthing_net::transport::RawTcpTransport::new()));
    manager.set_transport_registry(Arc::new(registry));

    let pending_responses = install_bep_bridge(&manager, &sync_service, connection_handle.clone());

    let block_source = Arc::new(TestBlockSource {
        manager: connection_handle.clone(),
        next_id: AtomicI32::new(1),
        pending_responses: pending_responses.clone(),
    });
    sync_service.set_block_source(block_source).await;

    // ponytail: add_folder 必须在 set_block_source 之后——FolderModel 仅在创建时
    // 读取 block_source（syncthing-sync lifecycle.rs add_folder_internal），
    // 否则 pull 报 "No block source configured"。
    let mut folder = Folder::new(&folder_id, folder_path.to_str().unwrap_or(""));
    folder.devices = peer_ids.clone();
    sync_service.add_folder(folder).await?;

    let actual_addr = manager.start().await?;
    println!("device_id={}", device_id);
    println!("listen_addr={}", actual_addr);
    println!("folder_id={}", folder_id);
    println!("folder_path={}", folder_path.display());

    tokio::signal::ctrl_c().await.ok();
    let _ = connection_handle.stop().await;
    let _ = sync_service.stop().await;
    Ok(())
}
