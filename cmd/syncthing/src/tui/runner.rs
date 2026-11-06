use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::{
    event::DisableMouseCapture,
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, BeginSynchronizedUpdate, DisableLineWrap,
        EnableLineWrap, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};

use super::app::App;
use super::constants;
use super::daemon_controller::{detect_external_daemon, toggle_daemon};
use super::events;
use super::log_loader::{find_latest_log_file, tail_lines};
use super::ui;
use super::TuiEvent;
use crate::logging_buffer::MemoryBuffer;

/// TUI entry point.
///
/// Mouse capture stays disabled because it misbehaves on Windows terminals.
///
/// See "docs/design/ADR-20261105-tui-first-frame-rendering.md" for the Windows
/// first-frame rendering workarounds applied here.
pub async fn run_tui(
    config_dir: PathBuf,
    listen: String,
    device_name: String,
    memory_buffer: MemoryBuffer,
    tray_pipe: Option<String>,
) -> anyhow::Result<()> {
    let config = crate::load_config(&config_dir.join(crate::CONFIG_FILE_NAME)).unwrap_or_default();

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        let _ = io::Write::flush(&mut io::stdout());
        original_hook(info);
    }));

    // Mouse capture stays disabled: it misbehaves on Windows terminals.
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        DisableLineWrap,
        BeginSynchronizedUpdate,
    )?;

    #[cfg(windows)]
    sync_console_buffer_size();

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Give Windows Terminal / conhost time to finish the alternate-screen switch
    // and size sync before the first frame.
    tokio::time::sleep(Duration::from_millis(100)).await;
    terminal.clear()?;

    let mut app = App::new(config_dir.clone(), listen, device_name, config);
    app.tray_pipe = tray_pipe;

    if let Some(pid) = detect_external_daemon(&app.config, &config_dir).await {
        app.daemon_running = true;
        app.external_daemon = true;
        app.external_daemon_pid = Some(pid);

        if let Some(pipe_name) = app.tray_pipe.clone() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                crate::tray_ipc::TrayIpcClient::connect(&pipe_name),
            )
            .await
            {
                Ok(Ok(mut client)) => {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        client.send(crate::tray_ipc::TrayIpcRequest::Ping),
                    )
                    .await
                    {
                        Ok(Ok(resp)) if resp.ok => {
                            app.tray_client = Some(client);
                            app.daemon_status = "Running (tray-managed)".to_string();
                            app.push_log(
                                "Tray-managed daemon detected. F5 controls daemon via tray."
                                    .to_string(),
                            );
                        }
                        _ => {
                            app.daemon_status = "Running (external)".to_string();
                            app.push_log(
                                "External daemon detected. F5 toggle disabled.".to_string(),
                            );
                        }
                    }
                }
                _ => {
                    app.daemon_status = "Running (external)".to_string();
                    app.push_log("External daemon detected. F5 toggle disabled.".to_string());
                }
            }
        } else {
            app.daemon_status = "Running (external)".to_string();
            app.push_log("External daemon detected. F5 toggle disabled.".to_string());
        }
    } else {
        app.push_log("TUI started. Press F5 to run daemon.".to_string());
    }

    // P1: TUI 启动时预加载历史日志
    let logs_dir = config_dir.join("logs");
    if logs_dir.is_dir() {
        if let Some(latest) = find_latest_log_file(&logs_dir) {
            match tail_lines(&latest, constants::LOG_TAIL_LINES) {
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

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        EnableLineWrap,
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
    let tick_rate = constants::TICK_RATE;
    let mut health_check_tick: u8 = 0;

    let mut daemon_join_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut daemon_handle: Option<syncthing_net::ConnectionManagerHandle> = None;
    let mut event_tx: Option<tokio::sync::mpsc::Sender<TuiEvent>> = None;
    let mut daemon_shutdown_tx: Option<tokio::sync::watch::Sender<bool>> = None;

    let mut stable_size = (0, 0);
    let mut stable_count = 0u8;
    for _i in 0..60 {
        let size = get_current_console_size();
        if size.0 > 0 && size.1 > 0 && size == stable_size {
            stable_count += 1;
            if stable_count >= 10 {
                break;
            }
        } else {
            stable_size = size;
            stable_count = 1;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if stable_size.0 > 0 && stable_size.1 > 0 {
        let _ = terminal.resize(ratatui::layout::Rect::new(
            0,
            0,
            stable_size.0,
            stable_size.1,
        ));
    }

    let _ = terminal.autoresize();
    let _ = terminal.clear();
    let _ = terminal.hide_cursor();
    let _ = terminal.draw(|f| ui::draw(f, app));
    let _ = terminal.flush();

    for _n in 0..2 {
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = terminal.autoresize();
        let _ = terminal.clear();
        let _ = terminal.draw(|f| ui::draw(f, app));
        let _ = terminal.flush();
    }

    let _ = execute!(io::stdout(), EndSynchronizedUpdate);

    trigger_startup_resize().await;
    let _ = terminal.resize(ratatui::layout::Rect::new(
        0,
        0,
        stable_size.0,
        stable_size.1,
    ));
    let _ = terminal.clear();
    let _ = terminal.draw(|f| ui::draw(f, app));
    let _ = terminal.flush();

    loop {
        terminal
            .draw(|f| ui::draw(f, app))
            .map_err(|e| io::Error::other(format!("{}", e)))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        let should_exit = if crossterm::event::poll(timeout)? {
            let event = crossterm::event::read()?;
            match &event {
                crossterm::event::Event::Resize(w, h) => {
                    let _ = terminal.resize(ratatui::layout::Rect::new(0, 0, *w, *h));
                    let _ = terminal.clear();
                    false
                }
                crossterm::event::Event::Key(key)
                    if key.code == crossterm::event::KeyCode::F(5)
                        && key.kind == crossterm::event::KeyEventKind::Press =>
                {
                    toggle_daemon(
                        app,
                        &mut daemon_join_handle,
                        &mut daemon_handle,
                        &mut event_tx,
                        &mut daemon_shutdown_tx,
                    )
                    .await;
                    false
                }
                _ => events::handle_event(app, &event),
            }
        } else {
            false
        };

        if should_exit {
            if !app.external_daemon {
                if let Some(tx) = daemon_shutdown_tx.take() {
                    let _ = tx.send(true);
                    app.push_log("Daemon shutdown signal sent.".to_string());
                    tokio::task::yield_now().await;
                }
                if let Some(handle) = daemon_join_handle.take() {
                    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
                }
                app.daemon_running = false;
                app.daemon_status = "Stopped".to_string();
            }
            app.event_rx = None;
            break;
        }

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
                        app.last_sync_progress_update = Some(Instant::now());
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            // P2: crash detection — a "running" daemon whose handle is dead or whose
            // main loop has finished is marked Crashed; external daemons are exempt.
            if app.daemon_running && !app.external_daemon {
                let alive = daemon_handle
                    .as_ref()
                    .map(|h| h.local_addr().is_some())
                    .unwrap_or(false);
                let finished = daemon_join_handle
                    .as_ref()
                    .map(|h| h.is_finished())
                    .unwrap_or(true);
                if !alive || finished {
                    app.daemon_running = false;
                    app.daemon_status = "Crashed".to_string();
                    app.push_log("Daemon crashed or exited unexpectedly.".to_string());
                    let _ = daemon_join_handle.take();
                    let _ = daemon_handle.take();
                    let _ = event_tx.take();
                    let _ = daemon_shutdown_tx.take();
                    app.sync_service = None;
                    app.event_rx = None;
                }
            }

            if !app.daemon_running
                && !app.external_daemon
                && app.daemon_status == "Stopping..."
                && daemon_join_handle
                    .as_ref()
                    .map(|h| h.is_finished())
                    .unwrap_or(true)
            {
                app.daemon_status = "Stopped".to_string();
                app.push_log("Daemon stopped.".to_string());
                let _ = daemon_join_handle.take();
            }

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
            for entry in memory_buffer.take_lines_filtered(100, &app.log_filter_level) {
                if app.log_lines.back().map(|s| s.as_str()) != Some(entry.msg.as_str()) {
                    app.push_log(entry.msg);
                }
            }

            if app.external_daemon {
                if let Some(pid) = app.external_daemon_pid {
                    if pid != 0 && !crate::single_instance::is_process_alive(pid) {
                        app.external_daemon = false;
                        app.external_daemon_pid = None;
                        app.tray_client = None;
                        app.daemon_running = false;
                        app.daemon_status = "Tray exited. Local control enabled.".to_string();
                        app.push_log(
                            "Tray process exited. TUI switched to local daemon control."
                                .to_string(),
                        );
                    }
                }
            }

            if app.tray_client.is_some() {
                health_check_tick = health_check_tick.wrapping_add(1);
                if health_check_tick.is_multiple_of(8) {
                    let alive = super::daemon_controller::daemon_health_check(&app.config).await;
                    if alive != app.daemon_running {
                        app.daemon_running = alive;
                        app.daemon_status = if alive {
                            "Running (tray-managed)".to_string()
                        } else {
                            "Stopped (tray-managed)".to_string()
                        };
                    }
                }
            }

            last_tick = tokio::time::Instant::now();
        }
    }

    Ok(())
}

/// Reads the real visible console window size (srWindow) via WinAPI.
///
/// `crossterm::terminal::size()` can return the screen-buffer size or a stale
/// size on Windows Terminal / ConPTY, which mislays the first frame; `srWindow`
/// reflects the rectangle the user actually sees.
#[cfg(windows)]
fn get_console_window_size() -> Option<(u16, u16)> {
    use windows::Win32::System::Console::{
        GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE,
    };

    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE).ok()?;
        let mut info = std::mem::zeroed();
        GetConsoleScreenBufferInfo(handle, &mut info).ok()?;
        let width = (info.srWindow.Right - info.srWindow.Left + 1) as u16;
        let height = (info.srWindow.Bottom - info.srWindow.Top + 1) as u16;
        if width > 0 && height > 0 {
            Some((width, height))
        } else {
            None
        }
    }
}

/// Syncs the console buffer size to the currently visible window size.
///
/// Prevents a scrollbar from appearing when the buffer is taller than the
/// window, which would push ratatui content below the visible area.
#[cfg(windows)]
fn sync_console_buffer_size() {
    use windows::Win32::System::Console::{
        GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleScreenBufferSize, COORD,
        STD_OUTPUT_HANDLE,
    };

    unsafe {
        let handle = match GetStdHandle(STD_OUTPUT_HANDLE) {
            Ok(h) => h,
            Err(e) => {
                tracing::debug!("Failed to get stdout handle for buffer sync: {}", e);
                return;
            }
        };
        let mut info = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(handle, &mut info).is_err() {
            return;
        }
        let width = info.srWindow.Right - info.srWindow.Left + 1;
        let height = info.srWindow.Bottom - info.srWindow.Top + 1;
        let _ = SetConsoleScreenBufferSize(
            handle,
            COORD {
                X: width,
                Y: height,
            },
        );
    }
}

/// Returns the currently visible console size, cross-platform.
///
/// On Windows prefers the WinAPI `srWindow` (the real visible rectangle) and
/// falls back to crossterm; other platforms use crossterm directly.
fn get_current_console_size() -> (u16, u16) {
    #[cfg(windows)]
    {
        get_console_window_size()
            .or_else(|| crossterm::terminal::size().ok())
            .unwrap_or((0, 0))
    }
    #[cfg(not(windows))]
    {
        crossterm::terminal::size().unwrap_or((0, 0))
    }
}

/// Triggers one real terminal resize at startup.
///
/// Some Windows terminals (Windows Terminal / conhost + ConPTY) mislay the
/// bottom row on the first frame after the alternate-screen switch, while a
/// user resize always fixes it. Shrinking the width by 1 column and restoring
/// it forces the terminal to reallocate the visible buffer and re-layout,
/// working around the first-frame corruption.
async fn trigger_startup_resize() {
    use crossterm::terminal::SetSize;

    let Ok((w, h)) = crossterm::terminal::size() else {
        return;
    };
    if w < 2 || h < 2 {
        return;
    }
    let _ = execute!(io::stdout(), SetSize(w - 1, h));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = execute!(io::stdout(), SetSize(w, h));
    tokio::time::sleep(Duration::from_millis(50)).await;
}
