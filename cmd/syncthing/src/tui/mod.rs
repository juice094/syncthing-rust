pub mod app;
pub mod bep_handler;
pub mod daemon_runner;
pub mod discovery_tasks;
pub mod events;
pub mod index_dispatcher;
pub mod nat_tasks;
pub mod popups;
pub mod relay_listener;
pub mod session_logger;
pub mod theme;
pub mod ui;
pub mod views;
pub mod widgets;

/// Sync engine → TUI 事件
#[derive(Debug, Clone)]
pub enum TuiEvent {
    FolderStateChanged {
        folder: String,
        status: syncthing_core::types::FolderStatus,
    },
    DeviceConnected {
        device_id: syncthing_core::DeviceId,
    },
    DeviceDisconnected {
        device_id: syncthing_core::DeviceId,
    },
    // TODO(v0.3.0): TUI real-time sync progress bar
    #[allow(dead_code)]
    SyncProgress {
        folder: String,
        progress: f64,
    },
}

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::{
    event::DisableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};
use tracing::warn;

use crate::logging_buffer::MemoryBuffer;
use app::App;

/// TUI 入口
pub async fn run_tui(
    config_dir: PathBuf,
    listen: String,
    device_name: String,
    memory_buffer: MemoryBuffer,
) -> anyhow::Result<()> {
    // 加载配置
    let config = crate::load_config(&config_dir.join(crate::CONFIG_FILE_NAME)).unwrap_or_default();

    // 设置 panic hook，确保终端在任何情况下都能恢复
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        let _ = io::Write::flush(&mut io::stdout());
        original_hook(info);
    }));

    // 设置终端（不启用鼠标捕获，避免 Windows 终端显示异常）
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new(config_dir.clone(), listen, device_name, config);
    app.push_log("TUI started. Press F5 to run daemon.".to_string());

    // P1: TUI 启动时预加载历史日志（最近 50 行）
    let logs_dir = config_dir.join("logs");
    if logs_dir.is_dir() {
        if let Some(latest) = find_latest_log_file(&logs_dir) {
            match tail_lines(&latest, 50) {
                Ok(lines) => {
                    for line in lines {
                        app.push_log(line);
                    }
                }
                Err(e) => app.push_log(format!("Failed to load log file: {}", e)),
            }
        }
    }

    let res = run_app(&mut terminal, &mut app, memory_buffer).await;

    // 恢复终端
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    terminal.clear()?;

    if let Err(err) = res {
        eprintln!("TUI error: {:?}", err);
    }

    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    memory_buffer: MemoryBuffer,
) -> io::Result<()> {
    let mut last_tick = tokio::time::Instant::now();
    let tick_rate = Duration::from_millis(250);

    let mut daemon_future: Option<
        std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>,
    > = None;
    let mut daemon_handle: Option<syncthing_net::ConnectionManagerHandle> = None;
    let mut event_tx: Option<tokio::sync::mpsc::Sender<TuiEvent>> = None;
    let mut daemon_shutdown_tx: Option<tokio::sync::watch::Sender<bool>> = None;

    loop {
        terminal
            .draw(|f| ui::draw(f, app))
            .map_err(|e| io::Error::other(format!("{}", e)))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        let should_exit = if crossterm::event::poll(timeout)? {
            let event = crossterm::event::read()?;
            match event {
                crossterm::event::Event::Resize(_, _) => {
                    // 窗口大小变化时立即重绘
                    false
                }
                crossterm::event::Event::Key(key)
                    if key.code == crossterm::event::KeyCode::F(5)
                        && key.kind == crossterm::event::KeyEventKind::Press =>
                {
                    toggle_daemon(
                        app,
                        &mut daemon_future,
                        &mut daemon_handle,
                        &mut event_tx,
                        &mut daemon_shutdown_tx,
                    )
                    .await;
                    false
                }
                _ => events::handle_event(app, event),
            }
        } else {
            false
        };

        if should_exit {
            // 优雅关闭 daemon（如果正在运行）
            if let Some(tx) = daemon_shutdown_tx.take() {
                let _ = tx.send(true);
                app.push_log("Daemon shutdown signal sent.".to_string());
                // 让出一次时间片让 daemon 处理 shutdown 信号，避免阻塞 TUI 退出
                tokio::task::yield_now().await;
            }
            app.daemon_running = false;
            app.daemon_status = "Stopped".to_string();
            app.event_rx = None;
            break;
        }

        // 接收 sync engine 事件
        if let Some(ref mut rx) = app.event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    TuiEvent::FolderStateChanged { folder, status } => {
                        app.folder_states.insert(folder, status);
                    }
                    TuiEvent::DeviceConnected { device_id } => {
                        if !app.connected_devices.contains(&device_id) {
                            app.connected_devices.push(device_id);
                        }
                    }
                    TuiEvent::DeviceDisconnected { device_id } => {
                        app.connected_devices.retain(|&id| id != device_id);
                    }
                    TuiEvent::SyncProgress { folder, progress } => {
                        app.sync_progress.insert(folder, progress);
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            // P2: daemon 崩溃检测 —— 如果 daemon_running 但 handle 已失效，标记为 Crashed
            if app.daemon_running {
                let alive = daemon_handle
                    .as_ref()
                    .map(|h| h.local_addr().is_some())
                    .unwrap_or(false);
                if !alive {
                    app.daemon_running = false;
                    app.daemon_status = "Crashed".to_string();
                    app.push_log("Daemon crashed or exited unexpectedly.".to_string());
                    let _ = daemon_future.take();
                    let _ = daemon_handle.take();
                    let _ = event_tx.take();
                    app.sync_service = None;
                    app.event_rx = None;
                }
            }

            // 轮询 daemon 状态（fallback，事件桥未覆盖时）
            if let Some(ref handle) = daemon_handle {
                let live = handle.connected_devices();
                for id in live {
                    if !app.connected_devices.contains(&id) {
                        app.connected_devices.push(id);
                    }
                }
                if app.daemon_running {
                    app.daemon_status = format!(
                        "Running | {} devices connected",
                        app.connected_devices.len()
                    );
                }
            }
            // 从内存日志缓冲区拉取新日志（按当前过滤级别）
            for entry in memory_buffer.take_lines_filtered(100, &app.log_filter_level) {
                // 避免重复追加已经存在的日志行（简单去重：检查最后一条）
                if app.log_lines.back().map(|s| s.as_str()) != Some(entry.msg.as_str()) {
                    app.push_log(entry.msg);
                }
            }
            last_tick = tokio::time::Instant::now();
        }
    }

    Ok(())
}

async fn toggle_daemon(
    app: &mut App,
    daemon_future: &mut Option<
        std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>,
    >,
    daemon_handle: &mut Option<syncthing_net::ConnectionManagerHandle>,
    event_tx: &mut Option<tokio::sync::mpsc::Sender<TuiEvent>>,
    daemon_shutdown_tx: &mut Option<tokio::sync::watch::Sender<bool>>,
) {
    // 使用 daemon_shutdown_tx 判断 daemon 是否运行（启动时设置，停止时 take）
    if daemon_shutdown_tx.is_some() {
        // 发送优雅关闭信号，然后清理本地状态
        if let Some(tx) = daemon_shutdown_tx.take() {
            let _ = tx.send(true);
        }
        *daemon_future = None;
        *daemon_handle = None;
        *event_tx = None;
        app.sync_service = None;
        app.event_rx = None;
        app.daemon_running = false;
        app.daemon_status = "Stopped".to_string();
        app.push_log("Daemon stopped.".to_string());
    } else {
        let config_dir = app.config_dir.clone();
        let listen = app.listen.clone();
        let device_name = app.device_name.clone();

        match daemon_runner::start_daemon(config_dir, listen, device_name).await {
            Ok(startup) => {
                *daemon_shutdown_tx = Some(startup.shutdown_tx.clone());
                *daemon_handle = Some(startup.connection_handle.clone());
                app.sync_service = Some(startup.sync_service.clone());

                // 启动事件桥
                let (tx, rx) = tokio::sync::mpsc::channel::<TuiEvent>(256);
                *event_tx = Some(tx.clone());
                app.event_rx = Some(rx);

                let sync_service = startup.sync_service.clone();
                tokio::spawn(async move {
                    let mut subscriber = sync_service.events().subscribe();
                    while let Some(event) = subscriber.recv().await {
                        let tui_event = match event {
                            syncthing_sync::SyncEvent::FolderStateChanged {
                                folder, to, ..
                            } => Some(TuiEvent::FolderStateChanged { folder, status: to }),
                            syncthing_sync::SyncEvent::DeviceConnected { device } => {
                                Some(TuiEvent::DeviceConnected { device_id: device })
                            }
                            syncthing_sync::SyncEvent::DeviceDisconnected { device, .. } => {
                                Some(TuiEvent::DeviceDisconnected { device_id: device })
                            }
                            syncthing_sync::SyncEvent::DownloadProgress {
                                folder,
                                file: _,
                                bytes_done,
                                bytes_total,
                            } => {
                                let progress = if bytes_total > 0 {
                                    bytes_done as f64 / bytes_total as f64
                                } else {
                                    0.0
                                };
                                Some(TuiEvent::SyncProgress { folder, progress })
                            }
                            _ => None,
                        };
                        if let Some(te) = tui_event {
                            if tx.send(te).await.is_err() {
                                break;
                            }
                        }
                    }
                });

                let fut = startup.future;
                tokio::spawn(async move {
                    if let Err(e) = fut.await {
                        warn!("Daemon exited with error: {}", e);
                    }
                });
                app.daemon_running = true;
                app.daemon_status = "Running".to_string();
                app.push_log("Daemon started.".to_string());
            }
            Err(e) => {
                app.popup = app::Popup::Error(format!("Failed to start daemon: {}", e));
                app.push_log(format!("Daemon start failed: {}", e));
            }
        }
    }
}

/// 在 logs 目录中找到最新的日志文件（按修改时间）
fn find_latest_log_file(logs_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(logs_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "log")
                .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));
    entries.into_iter().next().map(|e| e.path())
}

/// 读取文件最后 n 行
fn tail_lines(path: &std::path::Path, n: usize) -> anyhow::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let skip = lines.len().saturating_sub(n);
    Ok(lines.into_iter().skip(skip).collect())
}
