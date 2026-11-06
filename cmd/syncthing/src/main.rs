//! syncthing-rust binary entry point.
//!
//! Provides the CLI and daemon assembly. On Windows with the `tray` feature,
//! launching without arguments runs the daemon together with a system-tray
//! icon (Auto mode).
//!
//! See "docs/design/ADR-20261105-windows-tui-launch.md".

#![cfg_attr(all(windows, feature = "tray"), windows_subsystem = "windows")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing::{debug, info, warn, Level};
use tracing_subscriber::{layer::SubscriberExt, Layer as _};

use syncthing_core::types::Config;

use syncthing_sync::{BlockSource, RttTracker};

/// CLI default for listen address (references syncthing_core::constants).
const CLI_DEFAULT_LISTEN: &str = syncthing_core::constants::DEFAULT_LISTEN_ADDR;

/// Top-level command-line interface.
#[derive(Parser, Debug)]
#[command(name = "syncthing")]
#[command(about = "Syncthing Rust Implementation")]
struct Cli {
    /// Directory holding config.json and generated certificates.
    #[arg(long, global = true, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    /// Log level (error, warn, info, debug, trace).
    #[arg(short, long, global = true, default_value = "info")]
    log_level: String,

    /// Log format (text, json); json suits ELK/Splunk/Loki aggregation.
    #[arg(long, global = true, default_value = "text")]
    log_format: String,

    /// Subcommand; omitted = Auto mode (daemon + TUI, or daemon only without tray).
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the Syncthing daemon.
    Run {
        /// BEP listen address.
        #[arg(long, default_value = CLI_DEFAULT_LISTEN)]
        listen: String,

        /// Local device name.
        #[arg(short, long, default_value = syncthing_core::constants::DEFAULT_DEVICE_NAME)]
        device_name: String,
    },

    /// Launch the TUI config manager.
    Tui {
        /// BEP listen address.
        #[arg(long, default_value = CLI_DEFAULT_LISTEN)]
        listen: String,

        /// Local device name.
        #[arg(short, long, default_value = "syncthing-rust")]
        device_name: String,

        /// Tray IPC pipe name (internal use only, hidden).
        #[arg(long, hide = true)]
        tray_pipe: Option<String>,
    },

    /// Interactive wizard that generates config.json.
    Init,

    /// Query daemon runtime status.
    Status {
        /// Emit JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Device management.
    Devices {
        #[command(subcommand)]
        action: DeviceAction,
    },

    /// Folder management.
    Folders {
        #[command(subcommand)]
        action: FolderAction,
    },

    /// View daemon logs.
    Logs {
        /// Show the last N lines.
        #[arg(long, default_value = "50")]
        tail: usize,
    },

    /// Register autostart on login (Windows: HKCU Run key).
    InstallAutostart,

    /// Remove the autostart entry.
    UninstallAutostart,

    /// Run the built-in Relay Protocol server (Tailscale alternative).
    RelayServer {
        /// Protocol-mode listen address (TLS + bep-relay ALPN).
        #[arg(long, default_value = "0.0.0.0:22067")]
        listen: String,

        /// Session-mode port (same IP as `listen`).
        #[arg(long, default_value = "22068")]
        session_port: u16,
    },

    /// Auto mode: daemon + TUI (default behavior when no subcommand is given).
    #[command(hide = true)]
    Auto,
}

#[derive(Subcommand, Debug)]
enum DeviceAction {
    /// List configured devices.
    List,
}

#[derive(Subcommand, Debug)]
enum FolderAction {
    /// List configured folders.
    List {
        /// Include per-folder status information.
        #[arg(long)]
        status: bool,
    },
}

/// Name of the config file inside the config directory.
const CONFIG_FILE_NAME: &str = "config.json";

/// Loads a config from a JSON file.
fn load_config(path: &Path) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config from {:?}", path))?;
    let config: Config = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse config from {:?}", path))?;
    Ok(config)
}

/// Serializes `config` as JSON and writes it to `path`.
///
/// On unix the file is chmod 0o600 because it contains the API key and
/// device identity.
fn save_config(path: &Path, config: &Config) -> anyhow::Result<()> {
    let content = serde_json::to_string_pretty(config).context("failed to serialize config")?;
    std::fs::write(path, content)
        .with_context(|| format!("failed to write config to {:?}", path))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Failed to set restrictive permissions on config file"
            );
        }
    }

    Ok(())
}

mod api_client;
mod autostart;
mod cli_query;
mod cli_relay_server;
mod cli_status;
mod config_validation;
mod init_wizard;
mod logging;
mod logging_buffer;
mod single_instance;
#[cfg(all(windows, feature = "tray"))]
mod tray;
#[cfg(all(windows, feature = "tray"))]
mod tray_api;
mod tray_ipc;
mod tui;
use syncthing::api_server;

/// Guard that releases the single-instance pid file on drop.
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
/// `skip_path_check`: in TUI mode, skip the folder-path existence check (the
/// user may be in the middle of creating a folder).
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

    // C-UX-4: fail fast on invalid config before startup; in TUI mode degrade to a
    // warning because the TUI is the config editor and must open on broken configs.
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
    if let Err(_e) = run().await {
        #[cfg(windows)]
        show_error_message("Syncthing Error", &format!("{:#}", _e));
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("Failed to install rustls crypto provider"))?;

    let cli = Cli::parse();

    let log_level = cli
        .log_level
        .parse::<Level>()
        .context("invalid log level")?;
    let log_format = logging::LogFormat::from_str(&cli.log_format);

    let config_dir = cli
        .config_dir
        .unwrap_or_else(syncthing_core::paths::default_config_dir);

    // C-UX-5: single-instance lock is exclusive to the daemon; TUI/query commands never lock.
    let needs_lock = match &cli.command {
        None => true,
        Some(cmd) => !matches!(
            cmd,
            Commands::Tui { .. }
                | Commands::Status { .. }
                | Commands::Devices { .. }
                | Commands::Folders { .. }
                | Commands::Logs { .. }
                | Commands::InstallAutostart
                | Commands::UninstallAutostart
                | Commands::RelayServer { .. }
        ),
    };
    if needs_lock {
        if let Err(e) = single_instance::acquire(&config_dir) {
            if cli.command.is_none() {
                #[cfg(all(windows, feature = "tray"))]
                if request_tray_open_tui(&config_dir).await {
                    return Ok(());
                }
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

    let command = cli.command.unwrap_or(Commands::Auto);

    #[cfg(not(all(windows, feature = "tray")))]
    let command = match command {
        Commands::Auto => Commands::Run {
            listen: CLI_DEFAULT_LISTEN.to_string(),
            device_name: syncthing_core::constants::DEFAULT_DEVICE_NAME.to_string(),
        },
        other => other,
    };

    match command {
        Commands::Run {
            listen,
            device_name,
        } => {
            logging::init_daemon_logging(&config_dir, log_level, log_format)?;
            let (listen, device_name) =
                resolve_daemon_config(&config_dir, listen, device_name, false)?;
            match tui::daemon_runner::start_daemon(config_dir.clone(), listen, device_name).await {
                Ok(startup) => {
                    let (api_handle, api_addr) = match api_server::start_api_server(
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

                    // B1: Windows ConsoleCtrlEvent (console close / shutdown / logoff).
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
                                    windows::Win32::Foundation::BOOL(1),
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

                    let _watchdog_handle = crate::tui::watchdog::spawn(
                        api_addr,
                        config_dir.clone(),
                        startup.shutdown_tx.clone(),
                    );

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
        Commands::RelayServer {
            listen,
            session_port,
        } => {
            let listen_addr: std::net::SocketAddr = listen
                .parse()
                .unwrap_or_else(|_| "0.0.0.0:22067".parse().unwrap());
            if let Err(e) =
                cli_relay_server::run_relay_server(&config_dir, listen_addr, session_port).await
            {
                eprintln!("Relay server failed: {}", e);
                std::process::exit(1);
            }
        }
        #[cfg(all(windows, feature = "tray"))]
        Commands::Auto => {
            std::fs::create_dir_all(&config_dir)
                .with_context(|| format!("create config dir {}", config_dir.display()))?;

            logging::init_tray_logging(&config_dir, log_format);

            let (listen, device_name) = resolve_daemon_config(
                &config_dir,
                CLI_DEFAULT_LISTEN.to_string(),
                syncthing_core::constants::DEFAULT_DEVICE_NAME.to_string(),
                false,
            )?;

            let (daemon_cmd_tx, daemon_cmd_rx) = tokio::sync::mpsc::channel(4);
            let controller_config_dir = config_dir.clone();
            let controller_handle = tokio::spawn(async move {
                daemon_controller(controller_config_dir, listen, device_name, daemon_cmd_rx).await;
            });

            let tray_pipe_name = tray_ipc::generate_pipe_name();
            tracing::info!("Tray IPC server listening on {}", tray_pipe_name);
            let _ = tray_ipc::write_tray_pipe(&config_dir, &tray_pipe_name);
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
            let ipc_config_dir = config_dir.clone();
            let ipc_pipe_name = tray_pipe_name.clone();
            tokio::spawn(async move {
                while let Some(req) = tray_ipc_rx.recv().await {
                    match req {
                        tray_ipc::TrayIpcRequest::StartDaemon => {
                            let _ = daemon_cmd_tx_for_ipc.send(DaemonCommand::Start).await;
                        }
                        tray_ipc::TrayIpcRequest::StopDaemon => {
                            let _ = daemon_cmd_tx_for_ipc.send(DaemonCommand::Stop).await;
                        }
                        tray_ipc::TrayIpcRequest::OpenTui => {
                            spawn_tui_from_tray(&ipc_config_dir, &ipc_pipe_name);
                        }
                        tray_ipc::TrayIpcRequest::Ping => {}
                    }
                }
            });

            let _ = daemon_cmd_tx.send(DaemonCommand::Start).await;

            let (event_rx, tray_handle) = tray::spawn();

            for event in event_rx {
                tracing::debug!("Tray event: {:?}", event);
                match event {
                    tray::TrayEvent::OpenTui => spawn_tui_from_tray(&config_dir, &tray_pipe_name),
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
                        let _ = tray_handle.join();
                        tray_ipc::clear_tray_pipe(&config_dir);
                        break;
                    }
                }
            }

            let _ = controller_handle.await;
        }
        #[cfg(not(all(windows, feature = "tray")))]
        Commands::Auto => {
            unreachable!("Auto mode should have been converted to Run before matching")
        }
    }

    Ok(())
}

/// Commands accepted by the tray daemon controller.
#[cfg(all(windows, feature = "tray"))]
#[derive(Debug, Clone, Copy)]
enum DaemonCommand {
    Start,
    Stop,
    Exit,
}

/// Handles for one daemon + REST API service instance.
#[cfg(all(windows, feature = "tray"))]
struct DaemonServices {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    daemon_handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    api_handle: tokio::task::JoinHandle<()>,
    api_url: String,
}

/// Starts one daemon + REST API service instance.
///
/// Equivalent to the original Auto-mode wiring, but callable repeatedly so the
/// tray controller can restart the daemon on demand.
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

/// Daemon lifecycle controller.
///
/// Runs in its own tokio task, receives Start/Stop/Exit commands from the tray
/// event loop, owns the daemon + API instance, and mirrors the running state to
/// the tray thread.
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

/// Block source that fetches blocks from devices connected via the connection manager.
pub(crate) struct ManagerBlockSource {
    manager: syncthing_net::ConnectionManagerHandle,
    next_id: AtomicI32,
    pending_responses: std::sync::Arc<
        dashmap::DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>,
    >,
    rtt_tracker: Arc<RttTracker>,
}

impl ManagerBlockSource {
    /// Requests one block from the given device.
    ///
    /// Returns Ok when the device answers with a `NoError` response; Err covers
    /// send failure, timeout, or a non-zero remote error code. The response
    /// channel is registered before the request is sent, so a fast response can
    /// never arrive before the pending entry exists. The 120s timeout tolerates
    /// high-latency links (500ms RTT over Tailscale) with multi-block requests
    /// for large files in flight.
    async fn try_request_block_from_device(
        &self,
        device_id: syncthing_core::DeviceId,
        folder: &str,
        file: &str,
        block: &syncthing_core::types::BlockInfo,
        block_no: usize,
    ) -> syncthing_sync::Result<bytes::Bytes> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let start = Instant::now();

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

        self.rtt_tracker.sample(start.elapsed());
        debug!(
            "Block request RTT from {}: {} ms (avg {} ms)",
            device_id,
            start.elapsed().as_millis() as u64,
            self.rtt_tracker.rtt_ms()
        );

        Ok(bytes::Bytes::from(response.data))
    }

    /// Returns a handle to the RTT tracker.
    pub(crate) fn rtt_tracker(&self) -> Arc<RttTracker> {
        Arc::clone(&self.rtt_tracker)
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

/// Thin wrapper that launches the TUI config manager.
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

/// Runs the TUI in the current process.
///
/// Used by `Commands::Tui` and by Auto mode when another daemon instance holds
/// the single-instance lock. The TUI pins itself to the current OS thread via
/// `tokio::task::LocalSet` because crossterm on Windows keeps raw-mode state in
/// thread-locals that are lost if the future migrates threads.
async fn run_tui_mode(
    config_dir: &Path,
    listen: String,
    device_name: String,
    _log_level: Level,
    tray_pipe: Option<String>,
) -> Result<()> {
    #[cfg(windows)]
    ensure_console();

    let config_dir_buf = config_dir.to_path_buf();

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
            let _ = std::fs::remove_file(tui_pid_path(&config_dir_buf));
        }
    }

    write_tui_pid(&config_dir_buf);

    let local = tokio::task::LocalSet::new();
    let config_dir_for_local = config_dir_buf.clone();
    let result = local
        .run_until(async move {
            let memory_buffer = logging_buffer::MemoryBuffer::new(100);
            let memory_layer = logging_buffer::MemoryLayer::new(memory_buffer.clone());
            let (tui_file_writer, _tui_log_guard) =
                logging::init_tui_file_writer(&config_dir_for_local);
            let subscriber = tracing_subscriber::registry()
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(tui_file_writer)
                        .with_filter(tracing_subscriber::filter::LevelFilter::DEBUG),
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

/// Asks an already-running tray process to open/focus the TUI via its IPC pipe.
///
/// Returns true when the request was handed to the tray; returns false when no
/// tray exists or the pipe exchange failed, in which case the caller falls back
/// to launching a TUI in the current process.
#[cfg(all(windows, feature = "tray"))]
async fn request_tray_open_tui(config_dir: &Path) -> bool {
    let pipe_name = match tray_ipc::read_tray_pipe(config_dir) {
        Some(name) => name,
        None => {
            tracing::debug!("No tray pipe file found; falling back to local TUI");
            return false;
        }
    };

    let mut client = match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tray_ipc::TrayIpcClient::connect(&pipe_name),
    )
    .await
    {
        Ok(Ok(client)) => client,
        Ok(Err(e)) => {
            tracing::debug!("Failed to connect to tray IPC pipe: {}", e);
            return false;
        }
        Err(_) => {
            tracing::debug!("Timeout connecting to tray IPC pipe");
            return false;
        }
    };

    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client.send(tray_ipc::TrayIpcRequest::OpenTui),
    )
    .await
    {
        Ok(Ok(resp)) if resp.ok => {
            tracing::info!("Requested existing tray to open/focus TUI");
            true
        }
        Ok(Ok(resp)) => {
            tracing::debug!("Tray rejected OpenTui request: {:?}", resp.error);
            false
        }
        Ok(Err(e)) => {
            tracing::debug!("Failed to send OpenTui request: {}", e);
            false
        }
        Err(_) => {
            tracing::debug!("Timeout sending OpenTui request to tray");
            false
        }
    }
}

/// Polls the daemon REST API and mirrors status onto the tray icon.
///
/// Updates the tooltip, icon, and balloon notifications on online/offline and
/// device connect/disconnect transitions.
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

        // SAFETY: each tray API call is independently thread-safe (Shell_NotifyIconW
        // talks to the Shell via PostMessage). hwnd is read from the STATE OnceLock
        // and never changes after initialization.
        let hwnd = tray::tray_hwnd();
        if s.online && !last_online {
            // SAFETY: show_notification only calls Shell_NotifyIconW, which is thread-safe.
            unsafe {
                tray::show_notification(
                    hwnd,
                    "Syncthing",
                    "Daemon is online",
                    tray::BalloonIcon::Info,
                );
            }
        } else if !s.online && last_online {
            // SAFETY: same as above.
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
            // SAFETY: same as above.
            unsafe {
                tray::show_notification(hwnd, "Syncthing", &msg, tray::BalloonIcon::Info);
            }
        }

        if s.syncing && !last_syncing {
            // SAFETY: same as above.
            unsafe {
                tray::show_notification(hwnd, "Syncthing", "Sync started", tray::BalloonIcon::Info);
            }
        }

        // SAFETY: update_tooltip/update_icon are both Shell_NotifyIconW based and thread-safe.
        unsafe {
            tray::update_tooltip(hwnd, &tip);
            tray::update_icon(hwnd, icon);
        }

        last_online = s.online;
        last_connected = s.connected_devices;
        last_syncing = s.syncing;
    }
}

/// Windows: shows a fatal-error message box.
///
/// Required under `windows_subsystem = "windows"` because there is no console
/// window to print to.
#[cfg(windows)]
fn show_error_message(title: &str, message: &str) {
    use std::iter::once;
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title_wide: Vec<u16> = title.encode_utf16().chain(once(0)).collect();
    let msg_wide: Vec<u16> = message.encode_utf16().chain(once(0)).collect();

    // SAFETY: title_wide and msg_wide are null-terminated as required by MessageBoxW.
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(msg_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Windows: ensures the `windows_subsystem` binary has a usable console window.
///
/// Prefers attaching to the parent-process console (when launched from
/// PowerShell / Windows Terminal) so the TUI renders inside the launching
/// terminal; falls back to `AllocConsole()` when the parent has no console
/// (e.g. double-click from Explorer). Attaching shares input/output with the
/// parent, which is acceptable both for tray-spawned TUIs (the parent is a
/// dedicated `pwsh.exe`/`wt.exe`) and for `syncthing tui` run in a terminal.
#[cfg(windows)]
fn ensure_console() {
    use windows::Win32::System::Console::{
        AllocConsole, AttachConsole, GetConsoleWindow, SetConsoleTitleW, ATTACH_PARENT_PROCESS,
    };

    // SAFETY: the Win32 Console APIs are process-wide operations. AttachConsole /
    // AllocConsole attach/create a console only when the current process has
    // none; each is only needed once per process lifetime.
    unsafe {
        let existing = GetConsoleWindow();
        console_diag_log(format!(
            "ensure_console: existing_console_hwnd={:?}",
            existing.0
        ));
        if !existing.is_invalid() {
            console_diag_log("ensure_console: already has console, returning".to_string());
            return;
        }

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

/// Temporary diagnostics log for ensure_console (tracing is not initialized yet).
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

/// Windows: focuses and shows the current process's console window without
/// forcing a resize (window size follows the shell defaults; the TUI syncs the
/// screen buffer to the visible window in `runner.rs` via
/// `sync_console_buffer_size()`).
#[cfg(windows)]
unsafe fn configure_console_window() {
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_SHOWNORMAL};

    let hwnd = GetConsoleWindow();
    if !hwnd.is_invalid() {
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Spawns a TUI process from the tray.
///
/// Tries, in order: PowerShell 7 (`pwsh.exe`), Windows PowerShell, Windows
/// Terminal (`wt.exe`), and finally a bare `CreateProcess` with its own console.
/// All terminal launches go through `cmd /c start` so Microsoft Store
/// AppExecutionAlias binaries resolve (raw `CreateProcessW` cannot see them).
/// If a TUI is already running (pid file), its window is focused instead of
/// spawning a duplicate.
///
/// See "docs/design/ADR-20261105-windows-tui-launch.md" for the full rationale
/// and "docs/design/sequence/tray-tui-daemon.md" for the interaction sequence.
#[cfg(all(windows, feature = "tray"))]
fn spawn_tui_from_tray(config_dir: &Path, tray_pipe_name: &str) {
    if let Some(pid) = read_tui_pid(config_dir) {
        if is_process_alive(pid) {
            tracing::info!(
                "TUI already running (PID {}), focusing existing window",
                pid
            );
            focus_window_by_pid(pid);
            return;
        }
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
        let title = "syncthing-rust TUI";

        let exe_quoted = escape_ps_single_quote(&exe.to_string_lossy());
        let cd_quoted = escape_ps_single_quote(&cd);
        let pipe_quoted = escape_ps_single_quote(&pipe);
        let ps_cmd = format!(
            "Clear-Host; & '{}' tui --config-dir '{}' --tray-pipe '{}'",
            exe_quoted, cd_quoted, pipe_quoted
        );
        let encoded = encode_ps_command(&ps_cmd);

        if try_start_tui(&cd, title, "pwsh.exe", &["-EncodedCommand", &encoded]) {
            tracing::info!("TUI launched via PowerShell 7 (direct invoke)");
            return;
        }
        tracing::debug!("pwsh.exe not available or failed, trying Windows PowerShell");

        if try_start_tui(&cd, title, "powershell.exe", &["-EncodedCommand", &encoded]) {
            tracing::info!("TUI launched via Windows PowerShell (direct invoke)");
            return;
        }
        tracing::debug!("powershell.exe not available or failed, trying Windows Terminal");

        if try_start_wt_tui_direct(&cd, title, &exe, &pipe) {
            tracing::info!("TUI launched via Windows Terminal (direct)");
            return;
        }
        tracing::debug!(
            "wt.exe direct launch not available or failed, falling back to direct CreateProcess"
        );

        if try_start_tui_direct(&cd, &exe, &pipe) {
            tracing::info!("TUI launched via direct CreateProcess");
            return;
        }
        tracing::error!("All TUI launch strategies failed");
    });
}

/// Starts the TUI via `cmd /c start` and verifies that `syncthing-tui.pid`
/// actually appears, proving the terminal program really exists and launched
/// `syncthing.exe tui`. `cmd /c start` is used because it allocates a new
/// console for console programs and resolves Microsoft Store
/// AppExecutionAlias binaries that raw `CreateProcessW` cannot see.
#[cfg(all(windows, feature = "tray"))]
fn try_start_tui(config_dir: &str, title: &str, program: &str, args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;

    // CREATE_NO_WINDOW: the tray process itself has no console (windows_subsystem),
    // so a bare cmd.exe would flash a black console window before `start` runs.
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = std::process::Command::new("cmd.exe");
    cmd.arg("/c").arg("start").arg(title);
    cmd.arg(program);
    cmd.args(args);
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.status() {
        Ok(_) => {
            // `cmd /c start` returns immediately even when the target is missing,
            // so poll for the pid file. Windows Terminal cold start can take 2-3s,
            // hence a 5s budget to avoid a false fallback to the next terminal.
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

/// Starts the TUI in Windows Terminal directly, without a PowerShell
/// intermediate. Command shape:
/// `wt.exe new-tab --title <title> -- <exe> tui --config-dir <cd> --tray-pipe <pipe>`.
/// Still invoked via `cmd /c start` so the Microsoft Store AppExecutionAlias
/// for `wt.exe` resolves.
#[cfg(all(windows, feature = "tray"))]
fn try_start_wt_tui_direct(
    config_dir: &str,
    title: &str,
    exe: &std::path::Path,
    tray_pipe: &str,
) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let exe_str = exe.to_string_lossy();
    let mut cmd = std::process::Command::new("cmd.exe");
    cmd.arg("/c")
        .arg("start")
        .arg(title)
        .arg("wt.exe")
        .arg("new-tab")
        .arg("--title")
        .arg(title)
        .arg("--")
        .arg(&*exe_str)
        .arg("tui")
        .arg("--config-dir")
        .arg(config_dir)
        .arg("--tray-pipe")
        .arg(tray_pipe);
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.status() {
        Ok(_) => wait_for_tui_pid(config_dir, 50),
        Err(e) => {
            tracing::debug!("Failed to start wt.exe direct: {}", e);
            false
        }
    }
}

/// Starts the TUI via a bare `CreateProcess` with a fresh console window.
///
/// No intermediate shell, so it is the fastest strategy and cannot flash a
/// shell prompt; used as the last resort after all terminal launches failed.
#[cfg(all(windows, feature = "tray"))]
fn try_start_tui_direct(config_dir: &str, exe: &std::path::Path, tray_pipe: &str) -> bool {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_CONSOLE: the tray process has no console, so without this flag
    // the child would inherit none and the TUI would be invisible.
    const CREATE_NEW_CONSOLE: u32 = 0x00000010;

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("tui")
        .arg("--config-dir")
        .arg(config_dir)
        .arg("--tray-pipe")
        .arg(tray_pipe);
    cmd.creation_flags(CREATE_NEW_CONSOLE);

    match cmd.spawn() {
        Ok(_) => wait_for_tui_pid(config_dir, 30),
        Err(e) => {
            tracing::debug!("Failed to spawn TUI directly: {}", e);
            false
        }
    }
}

/// Waits for the TUI process to write its pid file, up to `ticks * 100ms`.
#[cfg(all(windows, feature = "tray"))]
fn wait_for_tui_pid(config_dir: &str, ticks: usize) -> bool {
    for _ in 0..ticks {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Some(pid) = read_tui_pid(std::path::Path::new(config_dir)) {
            if is_process_alive(pid) {
                return true;
            }
        }
    }
    tracing::debug!("tui.pid did not appear after waiting");
    false
}

/// Encodes a PowerShell command string as UTF-16 LE Base64 for `-EncodedCommand`.
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

/// Escapes single quotes for a PowerShell single-quoted string (`'` -> `''`).
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
    // SAFETY: OpenProcess with PROCESS_QUERY_LIMITED_INFORMATION is read-only and
    // safe for any PID. GetExitCodeProcess is read-only. CloseHandle must be
    // called to avoid a handle leak.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).unwrap_or_default();
        if handle.is_invalid() {
            return false;
        }
        let mut exit_code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut exit_code).is_ok();
        // SAFETY: handle was returned by OpenProcess and must be released via CloseHandle.
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

    // SAFETY: enum_proc is an EnumWindows callback invoked by the system with
    // valid hwnd/lparam. lparam points at the stack-allocated Ctx; EnumWindows
    // is synchronous so the reference stays valid for the whole call.
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
    // SAFETY: EnumWindows is a synchronous callback API. ctx lives on the stack
    // and remains valid for the whole call; LPARAM carries a &mut ctx pointer.
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    if !ctx.found.is_invalid() {
        // SAFETY: ctx.found comes from the EnumWindows callback after an
        // IsWindowVisible check, so the handle is valid and visible.
        // ShowWindow/SW_SHOWNORMAL restores a minimized window and
        // SetForegroundWindow brings it to the front; both are thread-safe.
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
