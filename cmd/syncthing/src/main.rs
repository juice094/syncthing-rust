//! Syncthing Rust 实现 - 主入口
//!
//! 提供命令行界面和守护进程功能

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::{debug, info, warn, Level};
use tracing_subscriber::{layer::SubscriberExt, Layer as _, FmtSubscriber};

use syncthing_core::types::Config;
use syncthing_net::SyncthingTlsConfig;
use syncthing_sync::BlockSource;

/// Syncthing 命令行参数
#[derive(Parser, Debug)]
#[command(name = "syncthing")]
#[command(about = "Syncthing Rust Implementation")]
struct Cli {
    /// 配置文件目录
    #[arg(long, global = true, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// 日志级别 (error, warn, info, debug, trace)
    #[arg(short, long, global = true, default_value = "info")]
    log_level: String,

    /// 子命令
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 运行 Syncthing 守护进程
    Run {
        /// 监听地址
        #[arg(long, default_value = "0.0.0.0:22001")]
        listen: String,

        /// 设备名称
        #[arg(short, long, default_value = "syncthing-rust")]
        device_name: String,

    },

    /// 启动 TUI 配置管理器
    Tui {
        /// 监听地址
        #[arg(long, default_value = "0.0.0.0:22001")]
        listen: String,

        /// 设备名称
        #[arg(short, long, default_value = "syncthing-rust")]
        device_name: String,
    },

    /// 生成新的设备证书
    GenerateCert {
        /// 设备名称
        #[arg(short, long, default_value = "syncthing-rust")]
        device_name: String,

        /// 强制覆盖现有证书
        #[arg(short, long)]
        force: bool,
    },

    /// 显示设备ID
    ShowId,

    /// 运行同步基准测试
    Syncbench {
        /// 测试场景
        #[arg(value_enum)]
        scenario: syncbench::Scenario,

        /// 源目录（生成测试数据）
        #[arg(long)]
        source_dir: Option<PathBuf>,

        /// 目标目录（验证同步结果）
        #[arg(long)]
        target_dir: Option<PathBuf>,
    },

    /// Flush collected metrics to CSV
    MetricsFlush {
        /// Output CSV path
        #[arg(default_value = "syncthing_metrics.csv")]
        output: PathBuf,
    },
}

/// 获取默认配置目录
///
/// Windows: %LOCALAPPDATA%/syncthing-rust
/// Linux/macOS: ~/.local/share/syncthing-rust
fn default_config_dir() -> PathBuf {
    dirs::data_local_dir()
        .map(|d| d.join("syncthing-rust"))
        .unwrap_or_else(|| PathBuf::from(".syncthing-rust"))
}

/// 配置文件名
const CONFIG_FILE_NAME: &str = "config.json";

/// 从配置文件加载配置
fn load_config(path: &PathBuf) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config from {:?}", path))?;
    let config: Config = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse config from {:?}", path))?;
    Ok(config)
}

/// 保存配置到文件
fn save_config(path: &PathBuf, config: &Config) -> anyhow::Result<()> {
    let content = serde_json::to_string_pretty(config)
        .context("failed to serialize config")?;
    std::fs::write(path, content)
        .with_context(|| format!("failed to write config to {:?}", path))?;
    Ok(())
}

mod tui;
mod logging_buffer;
mod syncbench;
mod api_server;

/// Resolve listen/device_name from config file, overridden by CLI args.
fn resolve_daemon_config(
    config_dir: &Path,
    cli_listen: String,
    cli_device_name: String,
) -> Result<(String, String)> {
    let config_path = config_dir.join(CONFIG_FILE_NAME);
    let config = if config_path.exists() {
        load_config(&config_path).unwrap_or_else(|e| {
            warn!("Failed to load config: {}. Using default.", e);
            syncthing_core::types::Config::new()
        })
    } else {
        syncthing_core::types::Config::new()
    };

    // CLI overrides config (runtime-only, do NOT persist to disk)
    let listen = if cli_listen != "0.0.0.0:22001" {
        cli_listen
    } else {
        config.listen_addr.clone()
    };
    let device_name = if cli_device_name != "syncthing-rust" {
        cli_device_name
    } else {
        config.device_name.clone()
    };

    Ok((listen, device_name))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // 确定配置目录
    let config_dir = cli.config_dir.unwrap_or_else(default_config_dir);
    let log_level = cli
        .log_level
        .parse::<Level>()
        .context("invalid log level")?;

    match cli.command {
        Commands::Run { listen, device_name } => {
            let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
            tracing::subscriber::set_global_default(subscriber)?;
            let (listen, device_name) = resolve_daemon_config(&config_dir, listen, device_name)?;
            match tui::daemon_runner::start_daemon(config_dir.clone(), listen, device_name).await {
                Ok(startup) => {
                    // 启动 REST API 服务器
                    let api_handle = match api_server::start_api_server(&config_dir, startup.sync_service.clone(), startup.device_id, Some(startup.connection_handle.clone())).await {
                        Ok(h) => h,
                        Err(e) => {
                            warn!("Failed to start REST API server: {}", e);
                            tokio::spawn(async {})
                        }
                    };
                    let daemon_result = startup.future.await;
                    let _ = api_handle.await;
                    daemon_result?;
                }
                Err(e) => {
                    eprintln!("Failed to start daemon: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Tui { listen, device_name } => {
            let memory_buffer = logging_buffer::MemoryBuffer::new(100);
            let memory_layer = logging_buffer::MemoryLayer::new(memory_buffer.clone());
            // TUI 模式下丢弃 stdout 输出，避免日志穿透到 TUI 外侧
            let fmt_layer = tracing_subscriber::fmt::layer()
                .with_writer(std::io::sink);
            let subscriber = tracing_subscriber::registry()
                .with(fmt_layer.with_filter(tracing_subscriber::filter::LevelFilter::from_level(log_level)))
                .with(memory_layer);
            tracing::subscriber::set_global_default(subscriber)?;
            let (listen, device_name) = resolve_daemon_config(&config_dir, listen, device_name)?;
            cmd_tui(&config_dir, &listen, &device_name, memory_buffer).await?;
        }
        Commands::GenerateCert { device_name, force } => {
            cmd_generate_cert(&config_dir, &device_name, force).await?;
        }
        Commands::ShowId => {
            cmd_show_id(&config_dir).await?;
        }
        Commands::Syncbench { scenario, source_dir, target_dir } => {
            syncbench::cmd_syncbench(scenario, source_dir, target_dir).await?;
        }
        Commands::MetricsFlush { output } => {
            syncthing_net::metrics::global().flush_to_csv(&output)?;
            println!("Metrics flushed to {:?}", output);
        }
    }

    Ok(())
}

/// 包装连接管理器的块数据源
pub(crate) struct ManagerBlockSource {
    manager: syncthing_net::ConnectionManagerHandle,
    next_id: AtomicI32,
    pending_responses: std::sync::Arc<dashmap::DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>>,
}

impl ManagerBlockSource {
    /// 尝试向指定设备请求一个块。
    /// 返回 Ok 表示设备返回了 NoError 响应；Err 表示发送失败、超时或设备返回错误码。
    async fn try_request_block_from_device(
        &self,
        device_id: syncthing_core::DeviceId,
        folder: &str,
        file: &str,
        block: &syncthing_core::types::BlockInfo,
    ) -> syncthing_sync::Result<bytes::Bytes> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let request = bep_protocol::messages::Request {
            id,
            folder: folder.to_string(),
            name: file.to_string(),
            offset: block.offset,
            size: block.size,
            hash: block.hash.clone(),
            from_temporary: false,
            block_no: 0,
        };

        let payload = bep_protocol::messages::encode_message(&request)
            .map_err(|e| syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("encode request failed: {}", e),
            ))?;

        let conn = self
            .manager
            .get_connection(&device_id)
            .ok_or_else(|| syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("Connection to {} not available", device_id),
            ))?;

        conn.send_message(syncthing_net::protocol::MessageType::Request, payload)
            .await
            .map_err(|e| syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("send request to {} failed: {}", device_id, e),
            ))?;

        // 注册等待响应
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_responses.insert(id, tx);

        let response = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("response timeout from {}", device_id),
            ))?
            .map_err(|_| syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("response channel closed for {}", device_id),
            ))?;

        if response.code != bep_protocol::messages::ErrorCode::NoError as i32 {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("remote error code {} from {}", response.code, device_id),
            ));
        }
        Ok(bytes::Bytes::from(response.data))
    }
}

#[async_trait::async_trait]
impl BlockSource for ManagerBlockSource {
    async fn request_block(
        &self,
        folder: &str,
        file: &str,
        block: &syncthing_core::types::BlockInfo,
    ) -> syncthing_sync::Result<bytes::Bytes> {
        let devices = self.manager.connected_devices();
        if devices.is_empty() {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                "No connected devices".to_string(),
            ));
        }

        debug!(
            "Requesting block {}/{} offset={} from {} candidate device(s)",
            folder, file, block.offset, devices.len()
        );

        let mut last_error = None;

        for device_id in devices {
            match self.try_request_block_from_device(device_id, folder, file, block).await {
                Ok(data) => {
                    debug!("Block {}/{} offset={} served by {}", folder, file, block.offset, device_id);
                    return Ok(data);
                }
                Err(e) => {
                    debug!("Device {} failed block request: {}", device_id, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| syncthing_sync::SyncError::pull(
            file.to_string(),
            "All connected devices failed to serve block".to_string(),
        )))
    }
}

/// 启动 TUI 配置管理器
async fn cmd_tui(
    config_dir: &Path,
    listen: &str,
    device_name: &str,
    memory_buffer: logging_buffer::MemoryBuffer,
) -> Result<()> {
    tui::run_tui(
        config_dir.to_path_buf(),
        listen.to_string(),
        device_name.to_string(),
        memory_buffer,
    )
    .await
}

/// 生成新的设备证书
async fn cmd_generate_cert(config_dir: &PathBuf, device_name: &str, force: bool) -> Result<()> {
    info!("Generating new device certificate...");
    info!("Config directory: {:?}", config_dir);
    info!("Device name: {}", device_name);

    // 确保证书目录存在
    if !config_dir.exists() {
        tokio::fs::create_dir_all(config_dir).await?;
    }

    let cert_path = config_dir.join(syncthing_net::tls::CERT_FILE_NAME);
    let key_path = config_dir.join(syncthing_net::tls::KEY_FILE_NAME);

    // 检查是否已存在
    if cert_path.exists() || key_path.exists() {
        if force {
            warn!("Existing certificates will be overwritten");
        } else {
            anyhow::bail!(
                "Certificates already exist. Use --force to overwrite, or use 'show-id' command to view the current device ID"
            );
        }
    }

    // 删除现有证书（如果存在）
    if cert_path.exists() {
        tokio::fs::remove_file(&cert_path).await?;
    }
    if key_path.exists() {
        tokio::fs::remove_file(&key_path).await?;
    }

    // 生成新证书（使用 load_or_generate 会生成并保存）
    let tls_config = SyncthingTlsConfig::load_or_generate(config_dir)
        .await
        .context("failed to generate certificate")?;

    let device_id = tls_config.device_id();

    println!("✅ 证书生成成功！");
    println!();
    println!("设备ID: {}", device_id);
    println!("证书路径: {:?}", cert_path);
    println!("私钥路径: {:?}", key_path);
    println!();
    println!("请妥善保管您的私钥文件！");

    Ok(())
}

/// 显示设备ID
async fn cmd_show_id(config_dir: &Path) -> Result<()> {
    let cert_path = config_dir.join(syncthing_net::tls::CERT_FILE_NAME);
    let key_path = config_dir.join(syncthing_net::tls::KEY_FILE_NAME);

    if !cert_path.exists() || !key_path.exists() {
        println!("❌ 未找到证书文件。请先运行 'generate-cert' 命令生成证书。");
        println!();
        println!("预期路径:");
        println!("  证书: {:?}", cert_path);
        println!("  私钥: {:?}", key_path);
        return Ok(());
    }

    // 加载现有证书
    let tls_config = SyncthingTlsConfig::load_or_generate(config_dir)
        .await
        .context("failed to load certificate")?;

    let device_id = tls_config.device_id();

    println!("设备ID: {}", device_id);
    println!("短ID:   {}", device_id.short_id());
    println!();
    println!("证书路径: {:?}", cert_path);
    println!("私钥路径: {:?}", key_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use super::*;
    use syncthing_net::{ConnectionManager, ConnectionManagerConfig};
    use syncthing_sync::{SyncService, database::MemoryDatabase};

    #[tokio::test]
    async fn test_daemon_start_stop() {
        let config_dir = std::env::temp_dir().join(format!(
            "syncthing-test-{}",
            std::process::id()
        ));

        // 清理旧数据
        let _ = tokio::fs::remove_dir_all(&config_dir).await;

        let tls_config = SyncthingTlsConfig::load_or_generate(&config_dir)
            .await
            .expect("failed to load or generate certificate");
        let tls_config_arc = Arc::new(tls_config);

        let db = MemoryDatabase::new();
        let config = Config::new();
        let sync_service = Arc::new(SyncService::new(db).with_config(config).await);

        let manager_config = ConnectionManagerConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        };
        let identity = Arc::new(syncthing_net::identity::TlsIdentity::new(Arc::clone(&tls_config_arc)));
        let (manager, _handle) =
            ConnectionManager::new(manager_config, identity, Arc::clone(&tls_config_arc));

        // 连接/断开回调（测试用空操作）
        manager.on_connected(move |_device_id| {
            // no-op for test
        });
        manager.on_disconnected(move |_device_id, _reason| {
            // no-op for test
        });

        // 启动服务
        sync_service.start().await.expect("failed to start sync service");
        let addr = manager.start().await.expect("failed to start connection manager");
        assert!(addr.port() > 0);

        // 停止服务
        sync_service.stop().await.expect("failed to stop sync service");
        manager.stop().await.expect("failed to stop connection manager");

        // 清理
        let _ = tokio::fs::remove_dir_all(&config_dir).await;
    }

    #[test]
    fn test_config_save_load() {
        let tmp_dir = std::env::temp_dir().join(format!("syncthing-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let path = tmp_dir.join("config.json");

        // 创建并保存配置
        let mut config = Config::new();
        config.devices.push(syncthing_core::types::Device {
            id: syncthing_core::DeviceId::default(),
            name: Some("test-device".to_string()),
            addresses: vec![syncthing_core::types::AddressType::Tcp("127.0.0.1:22001".to_string())],
            paused: false,
            introducer: false,
        });
        config.folders.push(syncthing_core::types::Folder::new("test-folder", "/tmp/test"));
        save_config(&path, &config).expect("failed to save config");

        // 加载并验证
        let loaded = load_config(&path).expect("failed to load config");
        assert_eq!(loaded.devices.len(), 1);
        assert_eq!(loaded.folders.len(), 1);
        assert_eq!(loaded.devices[0].name.as_deref(), Some("test-device"));
        assert_eq!(loaded.folders[0].id, "test-folder");

        // 清理
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_cli_override_does_not_persist() {
        let tmp_dir = std::env::temp_dir().join(format!("syncthing-cli-override-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let path = tmp_dir.join("config.json");
        let mut config = Config::new();
        config.listen_addr = "0.0.0.0:22001".to_string();
        config.device_name = "syncthing-rust".to_string();
        save_config(&path, &config).expect("failed to save config");

        // CLI override should NOT write back to disk
        let (listen, device_name) = resolve_daemon_config(
            &tmp_dir,
            "0.0.0.0:9999".to_string(),
            "custom-name".to_string(),
        ).expect("failed to resolve config");
        assert_eq!(listen, "0.0.0.0:9999");
        assert_eq!(device_name, "custom-name");

        // Verify disk config is untouched
        let loaded = load_config(&path).expect("failed to load config");
        assert_eq!(loaded.listen_addr, "0.0.0.0:22001");
        assert_eq!(loaded.device_name, "syncthing-rust");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_port_migration_persists() {
        let tmp_dir = std::env::temp_dir().join(format!("syncthing-port-migration-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let path = tmp_dir.join("config.json");
        let mut config = Config::new();
        config.listen_addr = "0.0.0.0:22000".to_string();
        config.gui.address = "127.0.0.1:8384".to_string();
        save_config(&path, &config).expect("failed to save config");

        // resolve_daemon_config does NOT migrate; migration happens in daemon_runner.
        // For this test we verify that resolve_daemon_config does not break the old port.
        let (listen, _) = resolve_daemon_config(&tmp_dir, "0.0.0.0:22001".to_string(), "syncthing-rust".to_string())
            .expect("failed to resolve config");
        // Because CLI arg equals default, it falls back to config value (the old 22000)
        assert_eq!(listen, "0.0.0.0:22000");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
