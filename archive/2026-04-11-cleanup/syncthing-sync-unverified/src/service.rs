//! Module: syncthing-sync
//! Worker: Agent-Integration
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! SyncService - 主同步服务
//!
//! 这是 Syncthing Rust 实现的主服务模块，负责协调所有组件：
//! - TCP 监听和连接接受
//! - 定期扫描调度
//! - 拉取调度
//!
//! 注意：API 服务在 cmd/syncthing 层组合，避免循环依赖


use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use syncthing_core::traits::{ConfigStore, Discovery, SyncModel, Transport};
use syncthing_core::types::{Config, DeviceId};
use syncthing_core::Result;
use syncthing_db::{create_block_store, CachedBlockStore};
use syncthing_fs::NativeFileSystem;
use syncthing_net::{ConnectionManager, TcpTransport, discovery::GlobalDiscoveryBuilder};

use crate::accept_loop::accept_loop;
use crate::model::SyncEngine;
use crate::scan_scheduler::ScanScheduler;

/// 主同步服务
///
/// 协调所有组件的主服务，包括：
/// - TCP 传输层
/// - 连接管理
/// - 同步引擎
/// - 扫描调度
pub struct SyncService {
    /// 配置
    config: RwLock<Config>,
    /// 配置存储
    #[allow(dead_code)]
    config_store: Arc<dyn ConfigStore>,
    /// 本地设备 ID
    device_id: DeviceId,
    /// TCP 传输层
    transport: Arc<TcpTransport>,
    /// 连接管理器
    connection_manager: Arc<ConnectionManager>,
    /// 同步引擎
    sync_engine: Arc<SyncEngine>,
    /// 扫描调度器
    scan_scheduler: Option<ScanScheduler>,
    /// 运行中的任务句柄
    task_handles: RwLock<Vec<JoinHandle<()>>>,
    /// 关闭信号发送器
    shutdown_tx: RwLock<Option<broadcast::Sender<()>>>,
    /// 关闭信号接收器
    _shutdown_rx: RwLock<broadcast::Receiver<()>>,
}

impl SyncService {
    /// 创建新的 SyncService
    ///
    /// # Arguments
    /// * `config` - 配置对象
    /// * `config_store` - 配置存储
    /// * `data_dir` - 数据目录（用于数据库等）
    ///
    /// # Returns
    /// 配置好的 SyncService 实例
    pub async fn new(
        config: Config,
        config_store: Arc<dyn ConfigStore>,
        data_dir: PathBuf,
    ) -> Result<(Self, DeviceId)> {
        info!("创建 SyncService...");

        // 创建 TCP 传输层（加载或生成证书）
        let cert_path = data_dir.join("cert.pem");
        let key_path = data_dir.join("key.pem");
        let transport = Arc::new(TcpTransport::new_with_cert_paths(&cert_path, &key_path)?);
        
        // 从证书获取本地设备 ID
        let device_id = transport.device_id();
        info!("本地设备ID: {} (从证书提取)", device_id.short_id());
        info!("TCP 传输层已创建");

        // 创建全球发现服务（容错：失败使用 NoopDiscovery）
        let discovery: Arc<dyn Discovery> = {
            let cert_path = data_dir.join("cert.pem");
            let key_path = data_dir.join("key.pem");
            
            let mut builder = GlobalDiscoveryBuilder::new();
            
            // 如果证书存在，使用证书认证
            if cert_path.exists() && key_path.exists() {
                builder = builder.with_certificate(
                    cert_path.to_string_lossy().to_string(),
                    key_path.to_string_lossy().to_string(),
                );
            }
            
            match builder.build().await {
                Ok(svc) => {
                    info!("全球发现服务已创建");
                    let d = Arc::new(svc);
                    // 启动发现服务 Announce 任务（忽略错误）
                    let local_addrs = vec![format!("tcp://0.0.0.0:22001")];
                    let _ = d.announce(&device_id, local_addrs).await;
                    let _ = d.start_periodic_announce(device_id, vec![format!("tcp://0.0.0.0:22001")], 1800).await;
                    d as Arc<dyn Discovery>
                }
                Err(e) => {
                    warn!("全球发现服务创建失败（将使用静态地址）: {}", e);
                    Arc::new(syncthing_core::traits::NoopDiscovery) as Arc<dyn Discovery>
                }
            }
        };

        // 创建连接管理器
        let connection_manager = Arc::new(ConnectionManager::new(
            transport.clone(),
            discovery.clone(),
            syncthing_net::manager::ConnectionConfig::default(),
        ));
        info!("连接管理器已创建");

        // 创建数据库目录
        let db_path = data_dir.join("database");
        tokio::fs::create_dir_all(&db_path).await.ok();

        // 创建块存储
        let block_store: Arc<CachedBlockStore> = Arc::new(create_block_store(&db_path)?);
        info!("块存储已创建: {:?}", db_path);

        // 创建文件系统
        let file_system: Arc<NativeFileSystem> = Arc::new(NativeFileSystem::new("/"));
        info!("文件系统已创建");

        // 创建同步引擎
        let sync_engine = Arc::new(SyncEngine::new(
            device_id,
            block_store.clone(),
            file_system.clone(),
            config_store.clone(),
            None, // TODO: 添加事件发布器
        ));
        info!("同步引擎已创建");

        // 添加配置的文件夹
        for folder_config in &config.folders {
            info!("添加文件夹: {} -> {:?}", folder_config.id, folder_config.path);
            if let Err(e) = sync_engine.add_folder(folder_config.clone()).await {
                warn!("添加文件夹失败: {} - {}", folder_config.id, e);
            }
        }

        // 创建扫描调度器
        let scan_scheduler = Some(ScanScheduler::new(
            config.folders.clone(),
            file_system.clone(),
            sync_engine.clone(),
        ));

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let service = Self {
            config: RwLock::new(config),
            config_store,
            device_id,
            transport,
            connection_manager,
            sync_engine,
            scan_scheduler,
            task_handles: RwLock::new(Vec::new()),
            shutdown_tx: RwLock::new(Some(shutdown_tx)),
            _shutdown_rx: RwLock::new(shutdown_rx),
        };
        Ok((service, device_id))
    }

    /// 启动所有服务
    ///
    /// 启动以下服务：
    /// 1. TCP 监听
    /// 2. 连接接受循环
    /// 3. 扫描调度
    pub async fn start(&mut self) -> Result<()> {
        info!("启动 SyncService...");

        let config = self.config.read().await.clone();

        // 1. 启动 TCP 监听
        let listen_addr = config
            .options
            .listen_addresses
            .first()
            .cloned()
            .unwrap_or_else(|| "tcp://0.0.0.0:22000".to_string());

        // 从地址中提取绑定地址
        let bind_addr = if listen_addr.starts_with("tcp://") {
            listen_addr.strip_prefix("tcp://").unwrap_or("0.0.0.0:22000")
        } else if listen_addr.starts_with("quic://") {
            // 暂时不支持 QUIC，回退到 TCP
            "0.0.0.0:22000"
        } else {
            &listen_addr
        };

        let listener = self.transport.listen(bind_addr).await?;
        info!("TCP 监听已启动: {}", bind_addr);

        // 2. 启动连接接受循环
        let sync_engine = self.sync_engine.clone();
        let accept_handle = tokio::spawn(async move {
            let mut listener = listener;
            accept_loop(&mut listener, sync_engine).await;
        });

        {
            let mut handles = self.task_handles.write().await;
            handles.push(accept_handle);
        }
        info!("连接接受循环已启动");

        // 3. 启动扫描调度
        if let Some(scheduler) = self.scan_scheduler.take() {
            let scan_handle = tokio::spawn(async move {
                scheduler.run().await;
            });
            let mut handles = self.task_handles.write().await;
            handles.push(scan_handle);
            info!("扫描调度器已启动");
        }

        // 4. 启动所有文件夹同步
        for folder_config in &config.folders {
            info!("启动文件夹同步: {}", folder_config.id);
            if let Err(e) = self.sync_engine.start_folder(folder_config.id.clone()).await {
                warn!("启动文件夹失败: {} - {}", folder_config.id, e);
            }
        }

        // 5. 连接到已知设备
        self.connect_to_devices(&config).await;

        // 6. 启动连接管理器的维护任务
        self.connection_manager.start_maintenance().await;

        info!("SyncService 启动完成");
        Ok(())
    }

    /// 连接到配置中已知设备
    async fn connect_to_devices(&self, config: &Config) {
        info!("连接到已知设备...");

        for device in &config.devices {
            if device.id == self.device_id {
                continue; // 跳过自己
            }

            for addr in &device.addresses {
                if addr == "dynamic" {
                    debug!("跳过动态地址设备: {}", device.id.short_id());
                    continue;
                }

                // 尝试连接
                let addr_clone = addr.clone();
                let device_id = device.id;
                let connection_manager = self.connection_manager.clone();
                let sync_engine = self.sync_engine.clone();

                tokio::spawn(async move {
                    match connection_manager.get_connection(&device_id).await {
                        Ok(conn) => {
                            info!("已连接到 {} at {}", device_id.short_id(), addr_clone);
                            // 让同步引擎处理这个连接
                            if let Err(e) = sync_engine.handle_connection(conn).await {
                                warn!("处理连接失败: {} - {}", device_id.short_id(), e);
                            }
                        }
                        Err(e) => {
                            debug!("连接失败 {} at {}: {}", device_id.short_id(), addr_clone, e);
                        }
                    }
                });
            }
        }
    }

    /// 停止所有服务
    pub async fn stop(&self) {
        info!("停止 SyncService...");

        // 发送关闭信号
        {
            let mut shutdown_tx = self.shutdown_tx.write().await;
            if let Some(tx) = shutdown_tx.take() {
                let _ = tx.send(());
            }
        }

        // 停止连接管理器的维护任务
        self.connection_manager.stop_maintenance().await;

        // 停止所有文件夹同步
        let config = self.config.read().await;
        for folder_config in &config.folders {
            info!("停止文件夹: {}", folder_config.id);
            if let Err(e) = self.sync_engine.stop_folder(folder_config.id.clone()).await {
                warn!("停止文件夹失败: {} - {}", folder_config.id, e);
            }
        }

        // 中止所有任务
        {
            let mut handles = self.task_handles.write().await;
            for handle in handles.drain(..) {
                handle.abort();
            }
        }

        info!("SyncService 已停止");
    }

    /// 获取同步引擎
    pub fn sync_engine(&self) -> Arc<SyncEngine> {
        self.sync_engine.clone()
    }

    /// 获取连接管理器
    pub fn connection_manager(&self) -> Arc<ConnectionManager> {
        self.connection_manager.clone()
    }

    /// 获取本地设备 ID
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// 等待关闭信号
    pub async fn wait_for_shutdown(&self) {
        let mut rx = {
            let shutdown_tx = self.shutdown_tx.read().await;
            if let Some(tx) = shutdown_tx.as_ref() {
                tx.subscribe()
            } else {
                // 如果没有发送器，创建一个永远不会收到的接收器
                let (_, rx) = broadcast::channel(1);
                rx
            }
        };

        let _ = rx.recv().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::{Compression, DeviceConfig, GuiConfig, Options};

    fn create_test_config() -> Config {
        let device_id = DeviceId::from_bytes([1u8; 32]);

        Config {
            version: 37,
            folders: vec![],
            devices: vec![DeviceConfig {
                id: device_id,
                name: "Test Device".to_string(),
                addresses: vec!["dynamic".to_string()],
                introducer: false,
                compression: Compression::Metadata,
            }],
            gui: GuiConfig {
                enabled: false,
                address: "127.0.0.1:8384".to_string(),
                api_key: None,
                use_tls: false,
            },
            options: Options {
                listen_addresses: vec!["tcp://127.0.0.1:0".to_string()],
                global_discovery: false,
                local_discovery: false,
                nat_traversal: false,
                relays_enabled: false,
            },
        }
    }

    #[tokio::test]
    async fn test_service_creation() {
        let config = create_test_config();
        // 使用内存配置存储进行测试
        let config_store = Arc::new(syncthing_api::config::MemoryConfigStore::new());
        let temp_dir = std::env::temp_dir().join("syncthing_test");

        let result = SyncService::new(config, config_store, temp_dir).await;
        assert!(result.is_ok());
        let (service, device_id) = result.unwrap();
        // 验证可以通过 service 获取相同的 device_id
        assert_eq!(service.device_id(), device_id);
    }
}
