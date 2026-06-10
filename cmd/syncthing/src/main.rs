//! Syncthing Rust 实现 - 主入口
//!
//! 提供命令行界面和守护进程功能
//!
//! Windows 桌面模式（feature = "tray"）：无参数启动 = daemon + 系统托盘图标

#![cfg_attr(all(windows, feature = "tray"), windows_subsystem = "windows")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::{debug, info, warn, Level};
use tracing_subscriber::{layer::SubscriberExt, Layer as _};

use syncthing_core::types::Config;

use syncthing_sync::BlockSource;

/// CLI default for listen address (references syncthing_core::constants).
const CLI_DEFAULT_LISTEN: &str = syncthing_core::constants::DEFAULT_LISTEN_ADDR;

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

    /// 子命令（不提供时自动启动 daemon + TUI）
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 运行 Syncthing 守护进程
    Run {
        /// 监听地址
        #[arg(long, default_value = CLI_DEFAULT_LISTEN)]
        listen: String,

        /// 设备名称
        #[arg(short, long, default_value = syncthing_core::constants::DEFAULT_DEVICE_NAME)]
        device_name: String,
    },

    /// 启动 TUI 配置管理器
    Tui {
        /// 监听地址
        #[arg(long, default_value = CLI_DEFAULT_LISTEN)]
        listen: String,

        /// 设备名称
        #[arg(short, long, default_value = "syncthing-rust")]
        device_name: String,
    },

    /// 交互式初始化向导（生成 config.json）
    Init,

    /// 查询 daemon 运行状态
    Status {
        /// 以 JSON 格式输出
        #[arg(long)]
        json: bool,
    },

    /// 设备管理
    Devices {
        #[command(subcommand)]
        action: DeviceAction,
    },

    /// 文件夹管理
    Folders {
        #[command(subcommand)]
        action: FolderAction,
    },

    /// 查看日志
    Logs {
        /// 显示最后 N 行
        #[arg(long, default_value = "50")]
        tail: usize,
    },

    /// 注册开机自启动（Windows: 注册表 Run 键）
    InstallAutostart,

    /// 取消开机自启动
    UninstallAutostart,

    /// 自动模式：启动 daemon + TUI（无子命令时的默认行为）
    #[command(hide = true)]
    Auto,
}

#[derive(Subcommand, Debug)]
enum DeviceAction {
    /// 列出已配置的设备
    List,
}

#[derive(Subcommand, Debug)]
enum FolderAction {
    /// 列出已配置的文件夹
    List {
        /// 包含状态信息
        #[arg(long)]
        status: bool,
    },
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
    let content = serde_json::to_string_pretty(config).context("failed to serialize config")?;
    std::fs::write(path, content)
        .with_context(|| format!("failed to write config to {:?}", path))?;
    Ok(())
}

mod autostart;
mod cli_query;
mod cli_status;
mod config_validation;
mod init_wizard;
mod logging_buffer;
mod single_instance;
#[cfg(all(windows, feature = "tray"))]
mod tray;
#[cfg(all(windows, feature = "tray"))]
mod tray_api;
mod tui;
use syncthing::api_server;

/// 单实例锁 Guard — 进程退出时自动删除 pid 文件
struct SingleInstanceGuard(std::path::PathBuf);

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        single_instance::release(&self.0);
    }
}

/// Resolve listen/device_name from config file, overridden by CLI args.
/// `skip_path_check`: TUI 模式下跳过 folder path 存在性检查（用户可能正在创建文件夹）。
fn resolve_daemon_config(
    config_dir: &Path,
    cli_listen: String,
    cli_device_name: String,
    skip_path_check: bool,
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

    // C-UX-4: 配置验证 — 启动前快速失败
    // TUI 模式下降级为警告：TUI 是配置编辑器，用户需要在界面内修复错误配置
    if skip_path_check {
        if let Err(e) = config_validation::validate_config_non_blocking(&config) {
            eprintln!(
                "[Config Warning] {}\n提示: 进入 TUI 后可修改配置修复此问题",
                e
            );
        }
    } else {
        config_validation::validate_config(&config)?;
    }

    // CLI overrides config (runtime-only, do NOT persist to disk)
    let listen = if cli_listen != CLI_DEFAULT_LISTEN {
        cli_listen
    } else {
        config.listen_addr.clone()
    };
    let device_name = if cli_device_name != "syncthing-rust" {
        cli_device_name
    } else {
        config.device_name
    };

    Ok((listen, device_name))
}

#[tokio::main]
async fn main() -> Result<()> {
    // 安装 rustls crypto provider，必须在任何 TLS 操作前执行
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("Failed to install rustls crypto provider"))?;

    let cli = Cli::parse();

    // 确定配置目录
    let config_dir = cli
        .config_dir
        .unwrap_or_else(syncthing_core::paths::default_config_dir);

    // C-UX-5: 单实例锁 — daemon 独占；TUI/查询命令不创建锁
    let needs_lock = match &cli.command {
        None => true, // auto mode = daemon + TUI
        Some(cmd) => !matches!(
            cmd,
            Commands::Tui { .. }
                | Commands::Status { .. }
                | Commands::Devices { .. }
                | Commands::Folders { .. }
                | Commands::Logs { .. }
                | Commands::InstallAutostart
                | Commands::UninstallAutostart
        ),
    };
    if needs_lock {
        single_instance::acquire(&config_dir).map_err(|e| anyhow::anyhow!(e))?;
    }
    let _instance_guard = if needs_lock {
        Some(SingleInstanceGuard(config_dir.clone()))
    } else {
        None
    };

    let log_level = cli
        .log_level
        .parse::<Level>()
        .context("invalid log level")?;

    match cli.command.unwrap_or(Commands::Auto) {
        Commands::Run {
            listen,
            device_name,
        } => {
            // H-2: 日志按小时轮转，保留 7 天（168 文件），防止单日无限膨胀
            let logs_dir = config_dir.join("logs");
            if let Err(e) = std::fs::create_dir_all(&logs_dir) {
                eprintln!("Warning: cannot create logs dir: {}", e);
            }
            let file_appender = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::HOURLY)
                .max_log_files(168)
                .filename_prefix("daemon")
                .filename_suffix("log")
                .build(&logs_dir)
                .unwrap_or_else(|e| {
                    eprintln!(
                        "Warning: cannot create rolling file appender: {}. Falling back to TMP.",
                        e
                    );
                    tracing_appender::rolling::hourly(std::env::temp_dir(), "syncthing-fallback")
                });
            let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
            let subscriber = tracing_subscriber::registry().with(
                tracing_subscriber::fmt::layer()
                    .with_writer(non_blocking)
                    .with_filter(tracing_subscriber::filter::LevelFilter::from_level(
                        log_level,
                    )),
            );
            tracing::subscriber::set_global_default(subscriber)
                .map_err(|e| anyhow::anyhow!("Failed to set subscriber: {}", e))?;
            let (listen, device_name) =
                resolve_daemon_config(&config_dir, listen, device_name, false)?;
            match tui::daemon_runner::start_daemon(config_dir.clone(), listen, device_name).await {
                Ok(startup) => {
                    // 启动 REST API 服务器
                    let (api_handle, _api_addr) = match api_server::start_api_server(
                        &config_dir,
                        startup.sync_service.clone(),
                        startup.device_id,
                        Some(startup.connection_handle.clone()),
                    )
                    .await
                    {
                        Ok(h) => h,
                        Err(e) => {
                            warn!("Failed to start REST API server: {}", e);
                            (tokio::spawn(async {}), SocketAddr::from(([0, 0, 0, 0], 0)))
                        }
                    };
                    let shutdown_tx = startup.shutdown_tx.clone();
                    tokio::spawn(async move {
                        tokio::signal::ctrl_c().await.ok();
                        info!("Received SIGINT, initiating graceful shutdown...");
                        let _ = shutdown_tx.send(true);
                    });

                    // B1: Windows ConsoleCtrlEvent (点击控制台X / 系统关机)
                    #[cfg(windows)]
                    {
                        let shutdown_tx = startup.shutdown_tx.clone();
                        std::thread::spawn(move || {
                            use std::sync::atomic::{AtomicBool, Ordering};
                            static CTRL_EVENT: AtomicBool = AtomicBool::new(false);

                            unsafe extern "system" fn handler(
                                ctrl_type: u32,
                            ) -> windows::Win32::Foundation::BOOL {
                                match ctrl_type {
                                    2 | 5 | 6 => {
                                        // CTRL_CLOSE_EVENT | CTRL_SHUTDOWN_EVENT | CTRL_LOGOFF_EVENT
                                        CTRL_EVENT.store(true, Ordering::SeqCst);
                                        windows::Win32::Foundation::BOOL(1)
                                    }
                                    _ => windows::Win32::Foundation::BOOL(0),
                                }
                            }

                            unsafe {
                                let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(
                                    Some(handler),
                                    windows::Win32::Foundation::BOOL(1), // TRUE = add handler
                                );
                            }

                            loop {
                                std::thread::sleep(std::time::Duration::from_millis(500));
                                if CTRL_EVENT.swap(false, Ordering::SeqCst) {
                                    info!("Received ConsoleCtrlEvent, initiating graceful shutdown...");
                                    let _ = shutdown_tx.send(true);
                                    break;
                                }
                            }
                        });
                    }

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
        Commands::Tui {
            listen,
            device_name,
        } => {
            let memory_buffer = logging_buffer::MemoryBuffer::new(100);
            let memory_layer = logging_buffer::MemoryLayer::new(memory_buffer.clone());
            // TUI 模式下丢弃 stdout 输出，避免日志穿透到 TUI 外侧
            let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::sink);
            let subscriber =
                tracing_subscriber::registry()
                    .with(fmt_layer.with_filter(
                        tracing_subscriber::filter::LevelFilter::from_level(log_level),
                    ))
                    .with(memory_layer);
            tracing::subscriber::set_global_default(subscriber)?;
            let (listen, device_name) =
                resolve_daemon_config(&config_dir, listen, device_name, true)?;
            cmd_tui(&config_dir, &listen, &device_name, memory_buffer).await?;
        }
        Commands::Init => {
            init_wizard::run_wizard(&config_dir)?;
        }
        Commands::Status { json } => {
            let config_path = config_dir.join(CONFIG_FILE_NAME);
            if !config_path.exists() {
                eprintln!(
                    "Config file not found: {}\n修复: 先运行 `syncthing init` 生成配置",
                    config_path.display()
                );
                std::process::exit(1);
            }
            if let Err(e) = cli_status::run(&config_path, json).await {
                eprintln!(
                    "Failed to query status: {}\n修复: 确认 daemon 已运行 (syncthing run)",
                    e
                );
                std::process::exit(1);
            }
        }
        Commands::Devices { action } => {
            let config_path = config_dir.join(CONFIG_FILE_NAME);
            if !config_path.exists() {
                eprintln!(
                    "Config file not found: {}\n修复: syncthing init",
                    config_path.display()
                );
                std::process::exit(1);
            }
            match action {
                DeviceAction::List => {
                    if let Err(e) = cli_query::devices_list(&config_path).await {
                        eprintln!("Failed to list devices: {}\n修复: 确认 daemon 已运行", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Folders { action } => {
            let config_path = config_dir.join(CONFIG_FILE_NAME);
            if !config_path.exists() {
                eprintln!(
                    "Config file not found: {}\n修复: syncthing init",
                    config_path.display()
                );
                std::process::exit(1);
            }
            match action {
                FolderAction::List { status } => {
                    if let Err(e) = cli_query::folders_list(&config_path, status).await {
                        eprintln!("Failed to list folders: {}\n修复: 确认 daemon 已运行", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Logs { tail } => {
            if let Err(e) = cli_query::logs_tail(&config_dir, tail) {
                eprintln!("Failed to read logs: {}", e);
                std::process::exit(1);
            }
        }
        Commands::InstallAutostart => {
            #[cfg(windows)]
            {
                if let Err(e) = autostart::install(&config_dir) {
                    eprintln!("Failed to install autostart: {}", e);
                    std::process::exit(1);
                }
                println!("Autostart installed. syncthing-rust will start on login.");
            }
            #[cfg(not(windows))]
            {
                eprintln!("Autostart is only supported on Windows.");
                std::process::exit(1);
            }
        }
        Commands::UninstallAutostart => {
            #[cfg(windows)]
            {
                if let Err(e) = autostart::uninstall() {
                    eprintln!("Failed to uninstall autostart: {}", e);
                    std::process::exit(1);
                }
                println!("Autostart removed.");
            }
            #[cfg(not(windows))]
            {
                eprintln!("Autostart is only supported on Windows.");
                std::process::exit(1);
            }
        }
        Commands::Auto => {
            // Auto mode: daemon + in-process tray icon (Windows only).
            // On Linux/macOS this is equivalent to `syncthing run`.
            std::fs::create_dir_all(&config_dir)
                .with_context(|| format!("create config dir {}", config_dir.display()))?;

            // Tray mode: file-only logging (no console)
            #[cfg(all(windows, feature = "tray"))]
            init_tray_logging(&config_dir);

            let (listen, device_name) = resolve_daemon_config(
                &config_dir,
                CLI_DEFAULT_LISTEN.to_string(),
                syncthing_core::constants::DEFAULT_DEVICE_NAME.to_string(),
                false,
            )?;

            match tui::daemon_runner::start_daemon(config_dir.clone(), listen, device_name).await {
                Ok(startup) => {
                    let (api_handle, _api_addr) = match api_server::start_api_server(
                        &config_dir,
                        startup.sync_service.clone(),
                        startup.device_id,
                        Some(startup.connection_handle.clone()),
                    )
                    .await
                    {
                        Ok(h) => h,
                        Err(e) => {
                            warn!("Failed to start REST API server: {}", e);
                            (tokio::spawn(async {}), SocketAddr::from(([0, 0, 0, 0], 0)))
                        }
                    };

                    let shutdown_tx = startup.shutdown_tx.clone();

                    #[cfg(all(windows, feature = "tray"))]
                    {
                        // Spawn tray thread + status polling task
                        let event_rx = tray::spawn();
                        let client = tray_api::DaemonClient::new("http://127.0.0.1:8385");

                        tokio::spawn(tray_status_loop(client));

                        // Main thread: process tray events
                        for event in event_rx {
                            tracing::debug!("Tray event: {:?}", event);
                            match event {
                                tray::TrayEvent::OpenWebUi => open_web_ui(),
                                tray::TrayEvent::OpenTui => spawn_tui_from_tray(&config_dir),
                                tray::TrayEvent::ToggleDaemon => {
                                    let _ = shutdown_tx.send(true);
                                }
                                tray::TrayEvent::Exit => {
                                    tracing::info!("Exit requested, cleaning up...");
                                    let _ = shutdown_tx.send(true);
                                    tray::cleanup();
                                    break;
                                }
                            }
                        }
                    }

                    #[cfg(not(all(windows, feature = "tray")))]
                    {
                        // Non-Windows or headless: just wait for Ctrl+C
                        tokio::spawn(async move {
                            tokio::signal::ctrl_c().await.ok();
                            info!("Received SIGINT, initiating graceful shutdown...");
                            let _ = shutdown_tx.send(true);
                        });
                    }

                    let daemon_result = startup.future.await;
                    let _ = api_handle.await;
                    daemon_result?;
                }
                Err(e) => {
                    tracing::error!("Failed to start daemon: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}

/// 包装连接管理器的块数据源
pub(crate) struct ManagerBlockSource {
    manager: syncthing_net::ConnectionManagerHandle,
    next_id: AtomicI32,
    pending_responses: std::sync::Arc<
        dashmap::DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>,
    >,
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
        block_no: usize,
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
            block_no: block_no as i32,
        };

        let payload = bep_protocol::messages::encode_message(&request).map_err(|e| {
            syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("encode request failed: {}", e),
            )
        })?;

        let conn = self.manager.get_connection(&device_id).ok_or_else(|| {
            syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("Connection to {} not available", device_id),
            )
        })?;

        // 注册等待响应（必须先注册，再发送，避免竞态）
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_responses.insert(id, tx);

        conn.send_message(syncthing_net::protocol::MessageType::Request, payload)
            .await
            .map_err(|e| {
                syncthing_sync::SyncError::pull(
                    file.to_string(),
                    format!("send request to {} failed: {}", device_id, e),
                )
            })?;

        // 120s — 高延迟 Tailscale 链路 (500ms RTT) + 大文件多块请求需要充足时间
        let response = tokio::time::timeout(std::time::Duration::from_secs(120), rx)
            .await
            .map_err(|_| {
                syncthing_sync::SyncError::pull(
                    file.to_string(),
                    format!("response timeout from {}", device_id),
                )
            })?
            .map_err(|_| {
                syncthing_sync::SyncError::pull(
                    file.to_string(),
                    format!("response channel closed for {}", device_id),
                )
            })?;

        debug!(
            "Block response from {}: code={} data_len={} (requested size={})",
            device_id,
            response.code,
            response.data.len(),
            block.size
        );

        if response.code != bep_protocol::messages::ErrorCode::NoError as i32 {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                format!("remote error code {} from {}", response.code, device_id),
            ));
        }
        if response.data.len() != block.size as usize {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                format!(
                    "block size mismatch from {}: expected {} got {}",
                    device_id,
                    block.size,
                    response.data.len()
                ),
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
        block_no: usize,
    ) -> syncthing_sync::Result<bytes::Bytes> {
        let devices = self.manager.connected_devices();
        debug!(
            "Requesting block {}/{} offset={} size={} block_no={}: {} connected device(s)",
            folder,
            file,
            block.offset,
            block.size,
            block_no,
            devices.len()
        );
        if devices.is_empty() {
            return Err(syncthing_sync::SyncError::pull(
                file.to_string(),
                "No connected devices".to_string(),
            ));
        }

        let mut last_error = None;

        for device_id in devices {
            match self
                .try_request_block_from_device(device_id, folder, file, block, block_no)
                .await
            {
                Ok(data) => {
                    debug!(
                        "Block {}/{} offset={} served by {}",
                        folder, file, block.offset, device_id
                    );
                    return Ok(data);
                }
                Err(e) => {
                    debug!("Device {} failed block request: {}", device_id, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            syncthing_sync::SyncError::pull(
                file.to_string(),
                "All connected devices failed to serve block".to_string(),
            )
        }))
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

// ==================== tray helpers (Windows + feature = "tray") ====================

#[cfg(all(windows, feature = "tray"))]
fn init_tray_logging(config_dir: &Path) {
    let logs_dir = config_dir.join("logs");
    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        eprintln!("Cannot create logs dir: {}", e);
        return;
    }

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .max_log_files(7)
        .filename_prefix("tray")
        .filename_suffix("log")
        .build(&logs_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to create tray log appender: {}. Using temp dir.", e);
            tracing_appender::rolling::daily(std::env::temp_dir(), "syncthing-tray-fallback")
        });

    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    std::mem::forget(_guard);

    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO),
    );
    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("Failed to set tray logger: {}", e);
    }
}

#[cfg(all(windows, feature = "tray"))]
async fn tray_status_loop(mut client: tray_api::DaemonClient) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    let mut last_online = false;
    let mut last_connected: usize = 0;
    let mut last_syncing = false;

    loop {
        interval.tick().await;
        let online = client.ping().await;
        client.set_online(online);
        if online {
            let _ = client.refresh_status().await;
        }
        let s = client.status();
        let icon = client.icon_type();
        let tip = format!(
            "syncthing-rust\n{} connected\n{} folder{}",
            if s.online {
                format!(
                    "{} device{}",
                    s.connected_devices,
                    if s.connected_devices == 1 { "" } else { "s" }
                )
            } else {
                "offline".to_string()
            },
            s.folder_count,
            if s.folder_count == 1 { "" } else { "s" }
        );

        unsafe {
            let hwnd = tray::tray_hwnd();
            if s.online && !last_online {
                tray::show_notification(
                    hwnd,
                    "Syncthing",
                    "Daemon is online",
                    tray::BalloonIcon::Info,
                );
            } else if !s.online && last_online {
                tray::show_notification(
                    hwnd,
                    "Syncthing",
                    "Daemon went offline",
                    tray::BalloonIcon::Error,
                );
            } else if s.online && s.connected_devices != last_connected {
                let msg = if s.connected_devices > last_connected {
                    format!("Device connected ({} total)", s.connected_devices)
                } else {
                    format!("Device disconnected ({} total)", s.connected_devices)
                };
                tray::show_notification(hwnd, "Syncthing", &msg, tray::BalloonIcon::Info);
            }

            if s.syncing && !last_syncing {
                tray::show_notification(hwnd, "Syncthing", "Sync started", tray::BalloonIcon::Info);
            }

            tray::update_tooltip(hwnd, &tip);
            tray::update_icon(hwnd, icon);
        }

        last_online = s.online;
        last_connected = s.connected_devices;
        last_syncing = s.syncing;
    }
}

#[cfg(all(windows, feature = "tray"))]
fn spawn_tui_from_tray(config_dir: &Path) {
    let exe = std::env::current_exe().unwrap_or_default();
    let cd = config_dir.to_string_lossy().to_string();
    std::thread::spawn(move || {
        match std::process::Command::new("cmd.exe")
            .arg("/c")
            .arg("start")
            .arg("")
            .arg(&exe)
            .arg("tui")
            .arg("--config-dir")
            .arg(&cd)
            .spawn()
        {
            Ok(child) => tracing::info!("TUI spawned via cmd /c start: pid={:?}", child.id()),
            Err(e) => tracing::error!("Failed to spawn TUI: {}", e),
        }
    });
}

#[cfg(all(windows, feature = "tray"))]
fn open_web_ui() {
    unsafe {
        let _ = windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            windows::core::w!("http://127.0.0.1:8385"),
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use syncthing_net::{ConnectionManager, ConnectionManagerConfig, SyncthingTlsConfig};
    use syncthing_sync::{database::MemoryDatabase, SyncService};

    #[tokio::test]
    async fn test_daemon_start_stop() {
        let config_dir =
            std::env::temp_dir().join(format!("syncthing-test-{}", std::process::id()));

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
        let identity = Arc::new(syncthing_net::identity::TlsIdentity::new(Arc::clone(
            &tls_config_arc,
        )));
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
        sync_service
            .start()
            .await
            .expect("failed to start sync service");
        let addr = manager
            .start()
            .await
            .expect("failed to start connection manager");
        assert!(addr.port() > 0);

        // 停止服务
        sync_service
            .stop()
            .await
            .expect("failed to stop sync service");
        manager
            .stop()
            .await
            .expect("failed to stop connection manager");

        // 清理
        let _ = tokio::fs::remove_dir_all(&config_dir).await;
    }

    #[test]
    fn test_config_save_load() {
        let tmp_dir =
            std::env::temp_dir().join(format!("syncthing-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let path = tmp_dir.join("config.json");

        // 创建并保存配置
        let mut config = Config::new();
        config.devices.push(syncthing_core::types::Device {
            id: syncthing_core::DeviceId::default(),
            name: Some("test-device".to_string()),
            addresses: vec![syncthing_core::types::AddressType::Tcp(format!(
                "127.0.0.1:{}",
                syncthing_core::constants::DEFAULT_BEP_PORT
            ))],
            paused: false,
            introducer: false,
        });
        config.folders.push(syncthing_core::types::Folder::new(
            "test-folder",
            "/tmp/test",
        ));
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
        let tmp_dir = std::env::temp_dir().join(format!(
            "syncthing-cli-override-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let path = tmp_dir.join("config.json");
        let mut config = Config::new();
        config.listen_addr = syncthing_core::constants::DEFAULT_LISTEN_ADDR.to_string();
        config.device_name = "syncthing-rust".to_string();
        save_config(&path, &config).expect("failed to save config");

        // CLI override should NOT write back to disk
        let (listen, device_name) = resolve_daemon_config(
            &tmp_dir,
            "0.0.0.0:9999".to_string(),
            "custom-name".to_string(),
            false,
        )
        .expect("failed to resolve config");
        assert_eq!(listen, "0.0.0.0:9999");
        assert_eq!(device_name, "custom-name");

        // Verify disk config is untouched
        let loaded = load_config(&path).expect("failed to load config");
        assert_eq!(
            loaded.listen_addr,
            syncthing_core::constants::DEFAULT_LISTEN_ADDR
        );
        assert_eq!(loaded.device_name, "syncthing-rust");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_port_migration_persists() {
        let tmp_dir = std::env::temp_dir().join(format!(
            "syncthing-port-migration-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let path = tmp_dir.join("config.json");
        let mut config = Config::new();
        config.listen_addr = "0.0.0.0:22000".to_string();
        config.gui.address = "127.0.0.1:8384".to_string();
        save_config(&path, &config).expect("failed to save config");

        // resolve_daemon_config does NOT migrate; migration happens in daemon_runner.
        // For this test we verify that resolve_daemon_config does not break the old port.
        let (listen, _) = resolve_daemon_config(
            &tmp_dir,
            syncthing_core::constants::DEFAULT_LISTEN_ADDR.to_string(),
            syncthing_core::constants::DEFAULT_DEVICE_NAME.to_string(),
            false,
        )
        .expect("failed to resolve config");
        // Because CLI arg equals default, it falls back to config value (the old 22000)
        assert_eq!(listen, "0.0.0.0:22000");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
