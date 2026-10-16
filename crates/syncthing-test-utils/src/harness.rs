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
use syncthing_sync::{
    database::{FileSystemDatabase, LocalDatabase, MemoryDatabase},
    SyncManager, SyncService,
};

use crate::bep_bridge::{install_bep_bridge, PendingResponses, SessionReadiness, TestBlockSource};

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
    /// ClusterConfig 就绪信号（对端 device_id → 对端通告的共享 folder 列表）。
    pub readiness: SessionReadiness,
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
    /// Uses an in-memory database; config is still persisted to config.json.
    pub async fn new_with_dir(name: &str, config_dir: PathBuf) -> Result<Self> {
        Self::new_with_dir_and_db(name, config_dir, MemoryDatabase::new()).await
    }

    /// Create and start a node with a specific config directory and a
    /// persistent file-system database. Used by the Android bridge so that
    /// index/version vectors survive service restarts.
    pub async fn new_with_dir_persistent(
        name: &str,
        config_dir: PathBuf,
        db_dir: PathBuf,
    ) -> Result<Self> {
        tokio::fs::create_dir_all(&db_dir)
            .await
            .context("create db dir")?;
        Self::new_with_dir_and_db(name, config_dir, FileSystemDatabase::new(&db_dir)).await
    }

    async fn new_with_dir_and_db(
        name: &str,
        config_dir: PathBuf,
        db: Arc<dyn LocalDatabase>,
    ) -> Result<Self> {
        tokio::fs::create_dir_all(&config_dir)
            .await
            .context("create config dir")?;

        // Generate certificate
        let tls_config = SyncthingTlsConfig::load_or_generate(&config_dir)
            .await
            .context("load_or_generate cert")?;
        let device_id = tls_config.device_id();
        let tls_config_arc = Arc::new(tls_config);

        // Load existing config if present (e.g., across service restarts),
        // otherwise build a fresh one. 证书已持久化，local_device_id 必须保持
        // 与证书一致，否则 device ID 会随重启变化。
        let config_path = config_dir.join("config.json");
        let config = if config_path.exists() {
            let bytes = tokio::fs::read(&config_path).await.context("read config")?;
            let mut cfg: Config = serde_json::from_slice(&bytes).context("parse config")?;
            // 证书是权威来源：用当前 device_id 覆盖 config 中可能存在的旧值/空值，
            // 保证 local_device_id 与证书一致。
            cfg.local_device_id = Some(device_id);
            cfg
        } else {
            let mut cfg = Config::new();
            cfg.device_name = name.to_string();
            cfg.listen_addr = "127.0.0.1:0".to_string();
            // 版本向量计数器 ID 来自 local_device_id；缺失会导致所有测试节点
            // 退化为 counter id 0，并发冲突检测失效（与 daemon_runner 对齐）
            cfg.local_device_id = Some(device_id);
            cfg.gui = GuiConfig {
                enabled: false,
                address: "127.0.0.1:0".to_string(),
                api_key: "e2e-test-key".to_string(),
                ro_api_key: String::new(),
            };
            cfg.options = Options {
                relays_enabled: false,
                ..Default::default()
            };
            cfg
        };

        // Persist config (for any downstream consumers that read it)
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
        let (pending_responses, readiness) =
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
            readiness,
            _manager: manager,
        })
    }

    /// Persist current in-memory config to config_dir/config.json.
    pub async fn persist_config(&self) -> Result<()> {
        let config = self.sync_service.get_config().await?;
        let path = self.config_dir.join("config.json");
        let json = serde_json::to_string_pretty(&config).context("serialize config")?;
        tokio::fs::write(&path, json)
            .await
            .with_context(|| format!("write config {}", path.display()))?;
        Ok(())
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
        self.persist_config().await?;
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
        self.persist_config().await?;
        Ok(())
    }

    /// 构造对端 `Device`（名称取对端配置目录名，地址取对端实际 BEP 监听地址）。
    fn peer_device(&self, peer: &TestNode) -> Device {
        Device {
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
        }
    }

    /// 只把对端登记进本节点设备表，**不发起拨号**。
    ///
    /// 供单边拨号测试使用：本节点作为监听方，靠对端拨入触发 `on_connected`。
    /// 双方同时拨号会在两端各产生一条 incoming + 一条 outgoing，触发连接竞争解决，
    /// 使失败方的 BepSession 阻塞到 10s ClusterConfig 超时（见 `docs/KNOWN_ISSUES.md` §1、§23）。
    pub async fn add_peer_device(&self, peer: &TestNode) -> Result<()> {
        let device = self.peer_device(peer);
        self.add_device(device).await
    }

    /// Configure connection to a peer node (add device + initiate connection).
    pub async fn connect_to(&self, peer: &TestNode) -> Result<()> {
        self.add_peer_device(peer).await?;
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
        self.add_peer_device(peer).await?;
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

    /// 等待与 `peer_id` 的 ClusterConfig 交换完成（即本节点已收到对端 ClusterConfig）。
    ///
    /// 这是 `wait_for_connection` 之上的握手就绪原语：连接建立 ≠ BEP 会话就绪，
    /// ClusterConfig 尚未交换时索引/块传输无人收发（见 `docs/KNOWN_ISSUES.md` §1、§23）。
    pub async fn wait_for_cluster_config(
        &self,
        peer_id: DeviceId,
        timeout: Duration,
    ) -> Result<()> {
        let what = format!("ClusterConfig from {}", peer_id);
        self.wait_for_ready(|r| r.contains_key(&peer_id), timeout, &what)
            .await
    }

    /// 等待 `peer_id` 在 ClusterConfig 中通告共享 `folder_id`（双方均可收发该 folder 的索引）。
    pub async fn wait_for_folder_ready(
        &self,
        peer_id: DeviceId,
        folder_id: &str,
        timeout: Duration,
    ) -> Result<()> {
        let what = format!("folder {} to be announced by {}", folder_id, peer_id);
        self.wait_for_ready(
            |r| {
                r.get(&peer_id)
                    .map(|folders| folders.iter().any(|id| id == folder_id))
                    .unwrap_or(false)
            },
            timeout,
            &what,
        )
        .await
    }

    /// 轮询就绪信号直到满足断言或超时；超时输出当前就绪表以便定位。
    async fn wait_for_ready<F>(&self, mut ready: F, timeout: Duration, what: &str) -> Result<()>
    where
        F: FnMut(&SessionReadiness) -> bool,
    {
        let start = std::time::Instant::now();
        while !ready(&self.readiness) {
            if start.elapsed() > timeout {
                let observed: Vec<(DeviceId, Vec<String>)> = self
                    .readiness
                    .iter()
                    .map(|e| (*e.key(), e.value().clone()))
                    .collect();
                anyhow::bail!("timeout waiting for {}; readiness={:?}", what, observed);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Graceful shutdown and cleanup of temporary directory.
    pub async fn shutdown(self) {
        let _ = self.connection_handle.stop().await;
        let _ = self.sync_service.stop().await;
        let _ = tokio::fs::remove_dir_all(&self.config_dir).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::Folder;

    #[tokio::test]
    async fn test_persistent_db_survives_restart() {
        let tmp = std::env::temp_dir().join(format!(
            "syncthing-persistent-db-test-{:x}",
            rand::random::<u64>()
        ));
        let config_dir = tmp.join("config");
        let db_dir = tmp.join("db");
        let folder_path = tmp.join("sync");
        tokio::fs::create_dir_all(&folder_path).await.unwrap();

        let folder_id = "obsidian-sync";

        // First run: scan a file into the persistent database.
        let node = TestNode::new_with_dir_persistent("persist", config_dir.clone(), db_dir.clone())
            .await
            .unwrap();
        let mut folder = Folder::new(folder_id, folder_path.to_str().unwrap());
        folder.rescan_interval_secs = 3600;
        node.add_folder(folder).await.unwrap();

        tokio::fs::write(folder_path.join("note.md"), b"hello")
            .await
            .unwrap();
        node.sync_service.scan_folder(folder_id).await.unwrap();
        node.wait_for_idle(folder_id, Duration::from_secs(10))
            .await
            .unwrap();

        let state = node.sync_service.get_folder_state(folder_id).await.unwrap();
        assert_eq!(state.local_files, 1, "first run should index one file");

        // Stop without cleanup so config/db persist.
        let _ = node.connection_handle.stop().await;
        let _ = node.sync_service.stop().await;
        drop(node);

        // Second run with the same directories: index must still be present.
        let node2 = TestNode::new_with_dir_persistent("persist", config_dir, db_dir)
            .await
            .unwrap();
        let state2 = node2
            .sync_service
            .get_folder_state(folder_id)
            .await
            .unwrap();
        assert_eq!(
            state2.local_files, 1,
            "persistent DB should retain index across restart"
        );

        // Clean up.
        let _ = node2.connection_handle.stop().await;
        let _ = node2.sync_service.stop().await;
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
