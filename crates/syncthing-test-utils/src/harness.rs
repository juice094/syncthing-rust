//! E2E test harness: programmable temporary Syncthing nodes.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicI32;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use syncthing_core::types::{Config, Device, Folder, FolderStatus, GuiConfig, Options};
use syncthing_core::DeviceId;
use syncthing_net::{
    identity::TlsIdentity, ConnectionManager, ConnectionManagerConfig, SyncthingTlsConfig,
};
use syncthing_sync::{database::MemoryDatabase, SyncManager, SyncService};

use crate::bep_bridge::{install_bep_bridge, PendingResponses, TestBlockSource};

/// Temporary test node.
pub struct TestNode {
    pub config_dir: PathBuf,
    pub device_id: DeviceId,
    /// Actual BEP listen address.
    pub bep_addr: SocketAddr,
    pub sync_service: Arc<SyncService>,
    pub connection_handle: syncthing_net::manager::ConnectionManagerHandle,
    /// T2.5 — Pending block-request responses, shared with `TestBlockSource`.
    /// Exposed for tests that want to inspect in-flight requests.
    pub pending_responses: PendingResponses,
    _manager: Arc<ConnectionManager>,
}

impl TestNode {
    /// Create and start a new temporary node.
    pub async fn new(name: &str) -> Result<Self> {
        let config_dir = std::env::temp_dir().join(format!(
            "syncthing-e2e-{}-{:x}",
            name,
            rand::random::<u64>()
        ));
        Self::new_with_dir(name, config_dir).await
    }

    /// Create and start a node with a specific config directory.
    pub async fn new_with_dir(name: &str, config_dir: PathBuf) -> Result<Self> {
        tokio::fs::create_dir_all(&config_dir)
            .await
            .context("create config dir")?;

        // Generate certificate
        let tls_config = SyncthingTlsConfig::load_or_generate(&config_dir)
            .await
            .context("load_or_generate cert")?;
        let device_id = tls_config.device_id();
        let tls_config_arc = Arc::new(tls_config);

        // In-memory database (sufficient for E2E single-process tests)
        let db = MemoryDatabase::new();

        // Build config
        let mut config = Config::new();
        config.device_name = name.to_string();
        config.listen_addr = "127.0.0.1:0".to_string();
        // 版本向量计数器 ID 来自 local_device_id；缺失会导致所有测试节点
        // 退化为 counter id 0，并发冲突检测失效（与 daemon_runner 对齐）
        config.local_device_id = Some(device_id);
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

        // Persist config (for any downstream consumers that read it)
        let config_path = config_dir.join("config.json");
        let config_json = serde_json::to_string_pretty(&config)?;
        tokio::fs::write(&config_path, config_json)
            .await
            .context("write config")?;

        // Start SyncService
        let sync_service = Arc::new(SyncService::new(db).with_config(config).await);
        sync_service.start().await.context("start sync service")?;

        // Start ConnectionManager
        let manager_config = ConnectionManagerConfig {
            listen_addr: "127.0.0.1:0".parse()?,
            ..Default::default()
        };
        let identity = Arc::new(TlsIdentity::new(Arc::clone(&tls_config_arc)));
        let (manager, connection_handle) =
            ConnectionManager::new(manager_config, identity, tls_config_arc);

        // Register TCP transport
        let mut registry = syncthing_net::transport::TransportRegistry::new();
        registry.register(Arc::new(syncthing_net::transport::RawTcpTransport::new()));
        manager.set_transport_registry(Arc::new(registry));

        // T2.5 — Install BEP session bridge BEFORE manager.start() so the
        // on_connected/on_disconnected callbacks fire for every peer connection.
        // This drives BepSession::run() per peer, enabling ClusterConfig /
        // Index / Block transfer (i.e. real end-to-end sync).
        let pending_responses =
            install_bep_bridge(&manager, &sync_service, connection_handle.clone());

        // Provide a block source so the puller can fetch remote blocks via BEP.
        let block_source = Arc::new(TestBlockSource {
            manager: connection_handle.clone(),
            next_id: AtomicI32::new(1),
            pending_responses: pending_responses.clone(),
        });
        sync_service.set_block_source(block_source).await;

        let bep_addr = manager.start().await.context("start connection manager")?;

        Ok(Self {
            config_dir,
            device_id,
            bep_addr,
            sync_service,
            connection_handle,
            pending_responses,
            _manager: manager,
        })
    }

    /// Add a folder (create local path and start sync).
    pub async fn add_folder(&self, folder: Folder) -> Result<()> {
        tokio::fs::create_dir_all(&folder.path)
            .await
            .with_context(|| format!("create folder path {}", folder.path))?;

        self.sync_service
            .add_folder(folder)
            .await
            .context("sync_service add_folder")?;
        Ok(())
    }

    /// Add a peer device to local config.
    pub async fn add_device(&self, device: Device) -> Result<()> {
        let mut config = self.sync_service.get_config().await?;
        config.devices.push(device);
        self.sync_service
            .update_config(config)
            .await
            .context("sync_service update_config")?;
        Ok(())
    }

    /// Configure connection to a peer node (add device + initiate connection).
    pub async fn connect_to(&self, peer: &TestNode) -> Result<()> {
        let device = Device {
            id: peer.device_id,
            name: Some(
                peer.config_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            ),
            addresses: vec![syncthing_core::types::AddressType::Tcp(format!(
                "tcp://{}",
                peer.bep_addr
            ))],
            paused: false,
            introducer: false,
        };
        self.add_device(device).await?;
        self.connection_handle
            .connect_to(peer.device_id, vec![peer.bep_addr])
            .await
            .context("connect_to peer")?;
        Ok(())
    }

    /// 强制重连到对等节点（断开现有连接后重新拨号）。
    ///
    /// 与 `connect_to` 不同，此方法在已连接时也会执行完整的重连流程，
    /// 确保 `on_connected` 被重新触发。
    pub async fn reconnect_to(&self, peer: &TestNode) -> Result<()> {
        let device = Device {
            id: peer.device_id,
            name: Some(
                peer.config_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            ),
            addresses: vec![syncthing_core::types::AddressType::Tcp(format!(
                "tcp://{}",
                peer.bep_addr
            ))],
            paused: false,
            introducer: false,
        };
        self.add_device(device).await?;
        self.connection_handle
            .reconnect_device(peer.device_id, vec![peer.bep_addr], vec![])
            .await
            .context("reconnect_to peer")?;
        Ok(())
    }

    /// Wait for connection to a specific device.
    pub async fn wait_for_connection(&self, peer_id: DeviceId, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();
        while self.connection_handle.get_connection(&peer_id).is_none() {
            if start.elapsed() > timeout {
                anyhow::bail!("timeout waiting for connection to {}", peer_id);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Wait for a folder to reach Idle state.
    pub async fn wait_for_idle(&self, folder_id: &str, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();
        loop {
            if let Some(folder) = self.sync_service.get_folder(folder_id) {
                let state = folder.state().await;
                if matches!(state.status, FolderStatus::Idle) {
                    return Ok(());
                }
            }
            if start.elapsed() > timeout {
                anyhow::bail!("timeout waiting for folder {} to become idle", folder_id);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Graceful shutdown and cleanup of temporary directory.
    pub async fn shutdown(self) {
        let _ = self.connection_handle.stop().await;
        let _ = self.sync_service.stop().await;
        let _ = tokio::fs::remove_dir_all(&self.config_dir).await;
    }
}
