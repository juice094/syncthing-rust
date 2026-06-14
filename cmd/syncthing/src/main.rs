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

        /// 托盘 IPC 管道名（仅内部使用，隐藏）
        #[arg(long, hide = true)]
        tray_pipe: Option<String>,
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
fn load_config(path: &Path) -> anyhow::Result<Config> {
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

mod api_client;
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
mod tray_ipc;
mod tui;
use syncthing::api_server;

/// 单实例锁 Guard — 进程退出时自动删除 pid 文件
struct SingleInstanceGuard(std::path::PathBuf);

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        single_instance::release(&self.0);
    }
}

const TUI_PID_FILE_NAME: &str = "syncthing-tui.pid";

fn tui_pid_path(config_dir: &Path) -> PathBuf {
    config_dir.join(TUI_PID_FILE_NAME)
}

fn write_tui_pid(config_dir: &Path) {
    let path = tui_pid_path(config_dir);
    if let Err(e) = std::fs::write(&path, format!("{}\n", std::process::id())) {
        tracing::warn!("Failed to write TUI PID file {}: {}", path.display(), e);
    }
}

fn cleanup_tui_pid(config_dir: &Path) {
    let _ = std::fs::remove_file(tui_pid_path(config_dir));
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
async fn main() {
    if let Err(e) = run().await {
        #[cfg(windows)]
        show_error_message("Syncthing Error", &format!("{:#}", e));
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    // 安装 rustls crypto provider，必须在任何 TLS 操作前执行
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("Failed to install rustls crypto provider"))?;

    let cli = Cli::parse();

    // 日志级别需要在单实例锁检查前解析，因为 Auto 模式检测到已有实例时会直接运行 TUI
    let log_level = cli
        .log_level
        .parse::<Level>()
        .context("invalid log level")?;

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
        if let Err(e) = single_instance::acquire(&config_dir) {
            // Auto 模式下若已有实例在运行，不弹窗，直接在当前进程打开 TUI。
            // 这样用户重复双击 syncthing.exe 会聚焦到 TUI，而不是报错退出。
            if cli.command.is_none() {
                return run_tui_mode(
                    &config_dir,
                    CLI_DEFAULT_LISTEN.to_string(),
                    syncthing_core::constants::DEFAULT_DEVICE_NAME.to_string(),
                    log_level,
                    None,
                )
                .await;
            }
            return Err(anyhow::anyhow!(e));
        }
    }
    let _instance_guard = if needs_lock {
        Some(SingleInstanceGuard(config_dir.clone()))
    } else {
        None
    };

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
                        startup.shutdown_tx.clone(),
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
            tray_pipe,
        } => {
            run_tui_mode(&config_dir, listen, device_name, log_level, tray_pipe).await?;
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

            // 启动 daemon 控制器（支持托盘内反复启停）
            let (daemon_cmd_tx, daemon_cmd_rx) = tokio::sync::mpsc::channel(4);
            let controller_config_dir = config_dir.clone();
            let controller_handle = tokio::spawn(async move {
                daemon_controller(controller_config_dir, listen, device_name, daemon_cmd_rx).await;
            });

            // 启动托盘 IPC 服务端，供从托盘打开的 TUI 发送 Start/Stop 命令
            let tray_pipe_name = tray_ipc::generate_pipe_name();
            tracing::info!("Tray IPC server listening on {}", tray_pipe_name);
            let (tray_ipc_tx, mut tray_ipc_rx) =
                tokio::sync::mpsc::channel::<tray_ipc::TrayIpcRequest>(4);
            let ipc_pipe_name_for_server = tray_pipe_name.clone();
            tokio::spawn(async move {
                let server = tray_ipc::TrayIpcServer::new(&ipc_pipe_name_for_server, tray_ipc_tx);
                if let Err(e) = server.run().await {
                    tracing::error!("Tray IPC server error: {}", e);
                }
            });
            let daemon_cmd_tx_for_ipc = daemon_cmd_tx.clone();
            tokio::spawn(async move {
                while let Some(req) = tray_ipc_rx.recv().await {
                    let cmd = match req {
                        tray_ipc::TrayIpcRequest::StartDaemon => Some(DaemonCommand::Start),
                        tray_ipc::TrayIpcRequest::StopDaemon => Some(DaemonCommand::Stop),
                        tray_ipc::TrayIpcRequest::Ping => None,
                    };
                    if let Some(cmd) = cmd {
                        let _ = daemon_cmd_tx_for_ipc.send(cmd).await;
                    }
                }
            });

            // 初始自动启动 daemon
            let _ = daemon_cmd_tx.send(DaemonCommand::Start).await;

            #[cfg(all(windows, feature = "tray"))]
            {
                let (event_rx, tray_handle) = tray::spawn();

                // Main thread: process tray events
                for event in event_rx {
                    tracing::debug!("Tray event: {:?}", event);
                    match event {
                        tray::TrayEvent::OpenTui => {
                            spawn_tui_from_tray(&config_dir, &tray_pipe_name)
                        }
                        tray::TrayEvent::ToggleDaemon => {
                            let cmd = if tray::daemon_running() {
                                DaemonCommand::Stop
                            } else {
                                DaemonCommand::Start
                            };
                            let _ = daemon_cmd_tx.send(cmd).await;
                        }
                        tray::TrayEvent::Exit => {
                            tracing::info!("Exit requested, cleaning up...");
                            let _ = daemon_cmd_tx.send(DaemonCommand::Exit).await;
                            tray::cleanup();
                            // 等待 tray 线程退出（WM_CLOSE → WM_DESTROY → PostQuitMessage → GetMessageW=0）
                            let _ = tray_handle.join();
                            break;
                        }
                    }
                }
            }

            #[cfg(not(all(windows, feature = "tray")))]
            {
                // Non-Windows or headless: just wait for Ctrl+C, then exit
                tokio::signal::ctrl_c().await.ok();
                info!("Received SIGINT, initiating graceful shutdown...");
                let _ = daemon_cmd_tx.send(DaemonCommand::Exit).await;
            }

            let _ = controller_handle.await;
        }
    }

    Ok(())
}

/// Daemon 控制器命令。
#[cfg(all(windows, feature = "tray"))]
#[derive(Debug, Clone, Copy)]
enum DaemonCommand {
    Start,
    Stop,
    Exit,
}

/// 一次 daemon + API 服务实例的句柄集合。
#[cfg(all(windows, feature = "tray"))]
struct DaemonServices {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    daemon_handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    api_handle: tokio::task::JoinHandle<()>,
    api_url: String,
}

/// 启动一组 daemon + REST API 服务。
///
/// 与 Auto 模式原始逻辑等价，但可被控制器重复调用以实现托盘内启停。
#[cfg(all(windows, feature = "tray"))]
async fn start_daemon_services(
    config_dir: &Path,
    listen: &str,
    device_name: &str,
) -> anyhow::Result<DaemonServices> {
    let startup = tui::daemon_runner::start_daemon(
        config_dir.to_path_buf(),
        listen.to_string(),
        device_name.to_string(),
    )
    .await?;

    let (api_handle, api_addr) = match api_server::start_api_server(
        config_dir,
        startup.sync_service.clone(),
        startup.device_id,
        Some(startup.connection_handle.clone()),
        startup.shutdown_tx.clone(),
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            warn!("Failed to start REST API server: {}", e);
            (tokio::spawn(async {}), SocketAddr::from(([0, 0, 0, 0], 0)))
        }
    };

    let api_url = if api_addr.port() != 0 {
        format!("http://127.0.0.1:{}", api_addr.port())
    } else {
        // 无法获取实际端口时回退到配置中的端口
        let config = load_config(&config_dir.join(CONFIG_FILE_NAME)).unwrap_or_default();
        let api_port = config.gui.address.rsplit(':').next().unwrap_or("8385");
        format!("http://127.0.0.1:{}", api_port)
    };

    let shutdown_tx = startup.shutdown_tx.clone();
    let daemon_handle = tokio::spawn(startup.future);

    Ok(DaemonServices {
        shutdown_tx,
        daemon_handle,
        api_handle,
        api_url,
    })
}

/// Daemon 生命周期控制器。
///
/// 运行在独立的 tokio task 中，接收来自托盘事件循环的 Start/Stop/Exit 命令，
/// 负责维护 daemon + API 实例并同步运行状态到托盘线程。
#[cfg(all(windows, feature = "tray"))]
async fn daemon_controller(
    config_dir: PathBuf,
    listen: String,
    device_name: String,
    mut cmd_rx: tokio::sync::mpsc::Receiver<DaemonCommand>,
) {
    let mut instance: Option<DaemonServices> = None;

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            DaemonCommand::Start => {
                if instance.is_some() {
                    continue;
                }
                match start_daemon_services(&config_dir, &listen, &device_name).await {
                    Ok(svc) => {
                        tray::set_daemon_running(true);
                        let client = tray_api::DaemonClient::new(&svc.api_url);
                        let status_shutdown = svc.shutdown_tx.subscribe();
                        tokio::spawn(async move {
                            tray_status_loop(client, status_shutdown).await;
                        });
                        instance = Some(svc);
                    }
                    Err(e) => {
                        warn!("Failed to start daemon: {}", e);
                        tray::set_daemon_running(false);
                    }
                }
            }
            DaemonCommand::Stop => {
                if let Some(svc) = instance.take() {
                    tray::set_daemon_running(false);
                    let _ = svc.shutdown_tx.send(true);
                    let _ = svc.daemon_handle.await;
                    let _ = svc.api_handle.await;
                    // 停止后将图标重置为默认离线状态
                    unsafe {
                        tray::update_icon(tray::tray_hwnd(), tray::IconType::Default);
                        tray::update_tooltip(tray::tray_hwnd(), "syncthing-rust\noffline");
                    }
                }
            }
            DaemonCommand::Exit => {
                if let Some(svc) = instance.take() {
                    tray::set_daemon_running(false);
                    let _ = svc.shutdown_tx.send(true);
                    let _ = svc.daemon_handle.await;
                    let _ = svc.api_handle.await;
                }
                break;
            }
        }
    }
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
    tray_pipe: Option<String>,
) -> Result<()> {
    tui::run_tui(
        config_dir.to_path_buf(),
        listen.to_string(),
        device_name.to_string(),
        memory_buffer,
        tray_pipe,
    )
    .await
}

/// 在当前进程运行 TUI 模式。
///
/// 供 `Commands::Tui` 和 Auto 模式检测到已有 daemon 实例时复用。
async fn run_tui_mode(
    config_dir: &Path,
    listen: String,
    device_name: String,
    log_level: Level,
    tray_pipe: Option<String>,
) -> Result<()> {
    #[cfg(windows)]
    ensure_console();

    let config_dir_buf = config_dir.to_path_buf();

    // TUI 单实例：若已有 TUI 在运行则聚焦其窗口并退出，避免重复打开。
    #[cfg(windows)]
    {
        if let Some(pid) = read_tui_pid(&config_dir_buf) {
            if is_process_alive(pid) {
                tracing::info!(
                    "TUI already running (PID {}), focusing existing window",
                    pid
                );
                focus_window_by_pid(pid);
                return Ok(());
            }
            // PID 文件残留，清理后继续
            let _ = std::fs::remove_file(tui_pid_path(&config_dir_buf));
        }
    }

    // 写入 TUI PID 文件，供托盘检测已存在实例
    write_tui_pid(&config_dir_buf);

    // LocalSet: 把 TUI 固定在当前 OS 线程，避免 crossterm 在 Windows 下
    // 因 future 跨线程调度而丢失 raw mode 的 thread-local 状态。
    let local = tokio::task::LocalSet::new();
    let config_dir_for_local = config_dir_buf.clone();
    let result = local
        .run_until(async move {
            let memory_buffer = logging_buffer::MemoryBuffer::new(100);
            let memory_layer = logging_buffer::MemoryLayer::new(memory_buffer.clone());
            // TUI 模式下 stdout 已被 crossterm 接管，因此把日志同时写入文件和内存缓冲区。
            let (tui_file_writer, _tui_log_guard) = init_tui_file_writer(&config_dir_for_local);
            let subscriber = tracing_subscriber::registry()
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(tui_file_writer)
                        .with_filter(tracing_subscriber::filter::LevelFilter::from_level(
                            log_level,
                        )),
                )
                .with(memory_layer);
            tracing::subscriber::set_global_default(subscriber)?;
            tracing::info!(
                "TUI started (config_dir={})",
                config_dir_for_local.display()
            );
            let (listen, device_name) =
                resolve_daemon_config(&config_dir_for_local, listen, device_name, true)?;
            cmd_tui(
                &config_dir_for_local,
                &listen,
                &device_name,
                memory_buffer,
                tray_pipe,
            )
            .await?;
            Ok::<(), anyhow::Error>(())
        })
        .await;

    cleanup_tui_pid(&config_dir_buf);
    result
}

// ==================== tray helpers (Windows + feature = "tray") ====================

/// 初始化 TUI 文件日志 appender，返回非阻塞 writer 与 guard。
///
/// Guard 需要被调用者持有，防止 tracing_appender 工作线程在 TUI 运行期间被 drop。
fn init_tui_file_writer(
    config_dir: &Path,
) -> (
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
) {
    let logs_dir = config_dir.join("logs");
    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        eprintln!("Cannot create TUI logs dir: {}", e);
    }

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .max_log_files(7)
        .filename_prefix("tui")
        .filename_suffix("log")
        .build(&logs_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to create TUI log appender: {}. Using temp dir.", e);
            tracing_appender::rolling::daily(std::env::temp_dir(), "syncthing-tui-fallback")
        });

    tracing_appender::non_blocking(file_appender)
}

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
    // SAFETY: std::mem::forget 防止 _guard 的 Drop 阻塞进程退出。
    // tracing_appender::non_blocking 的 guard 在 drop 时会 flush 缓冲区，
    // 但在托盘场景中进程退出由 Win32 消息驱动，drop 时机不可控。
    // forget 允许 OS 直接回收内存和线程，丢失少量日志是可接受的。
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
async fn tray_status_loop(
    mut client: tray_api::DaemonClient,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    let mut last_online = false;
    let mut last_connected: usize = 0;
    let mut last_syncing = false;

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    tracing::debug!("Tray status loop: daemon shut down, exiting");
                    return;
                }
            }
        }
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

        // SAFETY: 每个 tray API 调用都是独立线程安全的（Shell_NotifyIconW 通过 PostMessage 与 Shell 通信）。
        // hwnd 从 STATE OnceLock 读取，初始化后不变。
        let hwnd = tray::tray_hwnd();
        if s.online && !last_online {
            // SAFETY: show_notification 仅调用 Shell_NotifyIconW，线程安全。
            unsafe {
                tray::show_notification(
                    hwnd,
                    "Syncthing",
                    "Daemon is online",
                    tray::BalloonIcon::Info,
                );
            }
        } else if !s.online && last_online {
            // SAFETY: 同上。
            unsafe {
                tray::show_notification(
                    hwnd,
                    "Syncthing",
                    "Daemon went offline",
                    tray::BalloonIcon::Error,
                );
            }
        } else if s.online && s.connected_devices != last_connected {
            let msg = if s.connected_devices > last_connected {
                format!("Device connected ({} total)", s.connected_devices)
            } else {
                format!("Device disconnected ({} total)", s.connected_devices)
            };
            // SAFETY: 同上。
            unsafe {
                tray::show_notification(hwnd, "Syncthing", &msg, tray::BalloonIcon::Info);
            }
        }

        if s.syncing && !last_syncing {
            // SAFETY: 同上。
            unsafe {
                tray::show_notification(hwnd, "Syncthing", "Sync started", tray::BalloonIcon::Info);
            }
        }

        // SAFETY: update_tooltip/update_icon 均通过 Shell_NotifyIconW 实现，线程安全。
        unsafe {
            tray::update_tooltip(hwnd, &tip);
            tray::update_icon(hwnd, icon);
        }

        last_online = s.online;
        last_connected = s.connected_devices;
        last_syncing = s.syncing;
    }
}

/// Windows: 显示致命错误消息框。
///
/// 用于 `windows_subsystem = "windows"` 模式下的错误提示，因为此时没有控制台窗口。
#[cfg(windows)]
fn show_error_message(title: &str, message: &str) {
    use std::iter::once;
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title_wide: Vec<u16> = title.encode_utf16().chain(once(0)).collect();
    let msg_wide: Vec<u16> = message.encode_utf16().chain(once(0)).collect();

    // SAFETY: title_wide 和 msg_wide 均以 null 结尾，符合 MessageBoxW 要求。
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(msg_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Windows: 确保 `windows_subsystem` 二进制有一个可用的控制台窗口。
///
/// 优先附加到父进程控制台（当从 PowerShell / Windows Terminal 等终端启动时），
/// 这样 TUI 会渲染在启动它的终端窗口内，避免用户看到两个窗口（一个空终端、
/// 一个 TUI 控制台）。若父进程没有控制台（例如从资源管理器直接双击），则
/// 回退到 `AllocConsole()` 创建独立控制台窗口。
///
/// 注意：附加父控制台时，输入/输出会与父进程共享。托盘场景下父进程是
/// 专门为本 TUI 实例启动的 `pwsh.exe`/`wt.exe`，共享是可接受的；直接
/// 在终端中运行 `syncthing tui` 时，附加行为正好让 TUI 显示在当前标签页。
#[cfg(windows)]
fn ensure_console() {
    use windows::Win32::System::Console::{
        AllocConsole, AttachConsole, GetConsoleWindow, SetConsoleTitleW, ATTACH_PARENT_PROCESS,
    };

    // SAFETY: Win32 Console API 是进程级操作。AttachConsole/AllocConsole 在
    // 当前进程无控制台时附加/创建新控制台；这些 API 在进程生命周期内仅需调用一次。
    unsafe {
        // 如果已经附加到某个控制台（例如被显式启动在终端中），不再重复分配。
        let existing = GetConsoleWindow();
        console_diag_log(format!(
            "ensure_console: existing_console_hwnd={:?}",
            existing.0
        ));
        if !existing.is_invalid() {
            console_diag_log("ensure_console: already has console, returning".to_string());
            return;
        }

        // 优先附加到父进程控制台。托盘启动 TUI 时，父进程是 pwsh/wt，
        // 它们已经通过 cmd /c start 分配了控制台窗口。
        let attach_result = AttachConsole(ATTACH_PARENT_PROCESS);
        console_diag_log(format!(
            "ensure_console: AttachConsole result={:?}",
            attach_result
        ));
        if attach_result.is_ok() {
            let _ = SetConsoleTitleW(windows::core::w!("syncthing-rust TUI"));
            configure_console_window();
            return;
        }

        // 父进程没有控制台时，创建独立控制台窗口。
        let alloc_result = AllocConsole();
        console_diag_log(format!(
            "ensure_console: AllocConsole result={:?}",
            alloc_result
        ));
        if alloc_result.is_ok() {
            let _ = SetConsoleTitleW(windows::core::w!("syncthing-rust TUI"));
            configure_console_window();
        }
    }
}

/// 用于 ensure_console 诊断的临时日志（此时 tracing 尚未初始化）。
#[cfg(windows)]
fn console_diag_log(msg: String) {
    use std::io::Write;
    let path = std::env::temp_dir().join("syncthing-tui-console-diag.log");
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("[{}] {}\n", timestamp, msg);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

/// Windows: 调整并聚焦当前进程的控制台窗口。
#[cfg(windows)]
unsafe fn configure_console_window() {
    use windows::Win32::System::Console::{
        GetConsoleWindow, GetStdHandle, SetConsoleScreenBufferSize, SetConsoleWindowInfo, COORD,
        SMALL_RECT, STD_OUTPUT_HANDLE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_SHOWNORMAL};

    if let Ok(handle) = GetStdHandle(STD_OUTPUT_HANDLE) {
        let buf = COORD { X: 120, Y: 40 };
        let _ = SetConsoleScreenBufferSize(handle, buf);
        let rect = SMALL_RECT {
            Left: 0,
            Top: 0,
            Right: 119,
            Bottom: 39,
        };
        let _ = SetConsoleWindowInfo(handle, true, &rect);
    }

    let hwnd = GetConsoleWindow();
    if !hwnd.is_invalid() {
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// 从托盘启动 TUI。
///
/// 优先尝试 Windows Terminal / PowerShell 7 / Windows PowerShell 打开 TUI，
/// 使用终端原生窗口；均失败时回退到直接 CreateProcess + ensure_console()。
///
/// 对 wt.exe / pwsh.exe 使用 ShellExecuteW 启动，以正确处理 WindowsApps
///（Microsoft Store）安装产生的 AppExecutionAlias，避免 Rust 的 CreateProcess
/// 找不到可执行文件而直接回退到独立控制台窗口。
///
/// 启动前检查已有 TUI 实例（通过 PID 文件），若已存在则聚焦窗口而非重复创建。
#[cfg(all(windows, feature = "tray"))]
fn spawn_tui_from_tray(config_dir: &Path, tray_pipe_name: &str) {
    // 检查是否已有 TUI 实例在运行
    if let Some(pid) = read_tui_pid(config_dir) {
        if is_process_alive(pid) {
            tracing::info!(
                "TUI already running (PID {}), focusing existing window",
                pid
            );
            focus_window_by_pid(pid);
            return;
        }
        // PID 文件残留，清理后继续
        let _ = std::fs::remove_file(tui_pid_path(config_dir));
    }

    let exe = std::env::current_exe().unwrap_or_else(|e| {
        tracing::warn!(
            "Failed to get current exe path: {}. Using 'syncthing.exe'.",
            e
        );
        std::path::PathBuf::from("syncthing.exe")
    });
    let cd = config_dir.to_string_lossy().to_string();
    let pipe = tray_pipe_name.to_string();

    std::thread::spawn(move || {
        let exe_quoted = escape_ps_single_quote(&exe.to_string_lossy());
        let cd_quoted = escape_ps_single_quote(&cd);
        let pipe_quoted = escape_ps_single_quote(&pipe);
        // PowerShell 命令：使用 Start-Process -NoNewWindow 让 TUI 与 pwsh 共用同一个
        // 控制台窗口；Wait-Process 阻塞 pwsh 读取输入，避免 TUI 运行期间父 shell
        // 抢占键盘输入。使用 -EncodedCommand 避免 cmd/PowerShell 引号转义地狱。
        let ps_cmd = format!(
            "$proc = Start-Process -FilePath '{}' -ArgumentList 'tui','--config-dir','{}','--tray-pipe','{}' -NoNewWindow -PassThru; Wait-Process -Id $proc.Id",
            exe_quoted, cd_quoted, pipe_quoted
        );
        let encoded = encode_ps_command(&ps_cmd);
        let title = "syncthing-rust TUI";

        // 1. 优先 Windows Terminal：wt.exe 通过 cmd /c start 启动，以正确处理 WindowsApps alias
        if try_start_tui(
            &cd,
            title,
            "wt.exe",
            &[
                "new-tab",
                "--title",
                title,
                "--",
                "pwsh.exe",
                "-NoExit",
                "-EncodedCommand",
                &encoded,
            ],
        ) {
            tracing::info!("TUI launched via Windows Terminal");
            return;
        }
        tracing::debug!("wt.exe not available or failed, trying PowerShell 7");

        // 2. PowerShell 7：同样通过 cmd /c start，确保 pwsh 获得独立控制台窗口，
        //    并成为 TUI 的载体（TUI 渲染在 pwsh 窗口内）。
        if try_start_tui(
            &cd,
            title,
            "pwsh.exe",
            &["-NoExit", "-EncodedCommand", &encoded],
        ) {
            tracing::info!("TUI launched via PowerShell 7");
            return;
        }
        tracing::debug!("pwsh.exe not available or failed, trying Windows PowerShell");

        // 3. Windows PowerShell
        if try_start_tui(
            &cd,
            title,
            "powershell.exe",
            &["-NoExit", "-EncodedCommand", &encoded],
        ) {
            tracing::info!("TUI launched via Windows PowerShell");
            return;
        }
        tracing::debug!("powershell.exe not available or failed, falling back to direct spawn");

        // 4. 最终回退：直接 CreateProcess，由 ensure_console() 分配控制台
        let status = std::process::Command::new(&exe)
            .arg("tui")
            .arg("--config-dir")
            .arg(&cd)
            .arg("--tray-pipe")
            .arg(&pipe)
            .status();
        match status {
            Ok(s) => tracing::info!("TUI exited with status: {:?}", s.code()),
            Err(e) => tracing::error!("Failed to spawn TUI: {}", e),
        }
    });
}

/// 通过 `cmd /c start` 启动 TUI，并验证 `syncthing-tui.pid` 是否已写入，
/// 以确认目标终端程序真实存在并成功启动了 syncthing.exe tui。
///
/// 使用 `cmd /c start` 的原因：
/// - `start` 会为控制台程序分配新的控制台窗口；
/// - `cmd.exe` 能正确解析 WindowsApps（Microsoft Store）安装的 AppExecutionAlias，
///   而 Rust 的 `CreateProcessW` 直接调用 `pwsh.exe`/`wt.exe` 会找不到这些文件。
#[cfg(all(windows, feature = "tray"))]
fn try_start_tui(config_dir: &str, title: &str, program: &str, args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;

    // CREATE_NO_WINDOW: 托盘本身是 windows_subsystem（无控制台），
    // 直接启动 cmd.exe 会导致 cmd 先弹出一个黑色控制台窗口，再执行 start。
    // 加上此标志后 cmd 在后台执行，只保留 start 启动的目标终端窗口。
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = std::process::Command::new("cmd.exe");
    cmd.arg("/c").arg("start").arg(title);
    cmd.arg(program);
    cmd.args(args);
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.status() {
        Ok(_) => {
            // cmd /c start 会立即返回，即使目标程序不存在也会成功。
            // 等待 TUI 真实启动：Windows Terminal 冷启动可能需要 2~3 秒，
            // 因此把超时从 1 秒延长到 5 秒，避免误判为失败并回退到第二个终端。
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if let Some(pid) = read_tui_pid(std::path::Path::new(config_dir)) {
                    if is_process_alive(pid) {
                        return true;
                    }
                }
            }
            tracing::debug!("{} started but tui.pid did not appear", program);
            false
        }
        Err(e) => {
            tracing::debug!("Failed to start {} via cmd /c start: {}", program, e);
            false
        }
    }
}

/// 将 PowerShell 命令字符串编码为 UTF-16 LE Base64，供 `-EncodedCommand` 使用。
#[cfg(all(windows, feature = "tray"))]
fn encode_ps_command(cmd: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let utf16: Vec<u16> = cmd.encode_utf16().collect();
    let mut bytes = Vec::with_capacity(utf16.len() * 2);
    for unit in utf16 {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    STANDARD.encode(&bytes)
}

/// 将字符串中的单引号替换为两个单引号，供 PowerShell 单引号字符串使用。
#[cfg(all(windows, feature = "tray"))]
fn escape_ps_single_quote(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(windows)]
fn read_tui_pid(config_dir: &Path) -> Option<u32> {
    let path = tui_pid_path(config_dir);
    std::fs::read_to_string(&path)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // STILL_ACTIVE = STATUS_PENDING = 259
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: OpenProcess with PROCESS_QUERY_LIMITED_INFORMATION is read-only and safe for any PID.
    // GetExitCodeProcess is read-only. CloseHandle must be called to avoid handle leak.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).unwrap_or_default();
        if handle.is_invalid() {
            return false;
        }
        let mut exit_code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut exit_code).is_ok();
        // SAFETY: handle 由 OpenProcess 返回，必须通过 CloseHandle 释放。
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        ok && exit_code == STILL_ACTIVE
    }
}

#[cfg(windows)]
fn focus_window_by_pid(target_pid: u32) {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow, ShowWindow,
        SW_SHOWNORMAL,
    };

    struct Ctx {
        target_pid: u32,
        found: HWND,
    }

    // SAFETY: enum_proc 作为 EnumWindows 回调，由系统以正确的 hwnd/lparam 调用。
    // lparam 指向调用栈上的 Ctx 结构体，EnumWindows 是同步 API，生命周期有效。
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == ctx.target_pid && IsWindowVisible(hwnd).as_bool() {
            ctx.found = hwnd;
            return BOOL::from(false);
        }
        BOOL::from(true)
    }

    let mut ctx = Ctx {
        target_pid,
        found: HWND::default(),
    };
    // SAFETY: EnumWindows 是同步回调 API。ctx 在调用栈上，回调期间有效。
    // LPARAM 传递 &mut ctx 指针。
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    if !ctx.found.is_invalid() {
        // SAFETY: ctx.found 来自 EnumWindows 回调中的 IsWindowVisible 检查，
        // 确认窗口句柄有效且可见。ShowWindow/SW_SHOWNORMAL 恢复最小化窗口，
        // SetForegroundWindow 将窗口带到前台。这些都是线程安全的。
        unsafe {
            let _ = ShowWindow(ctx.found, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(ctx.found);
        }
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
