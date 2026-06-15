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

/// TUI 入口
pub async fn run_tui(
    config_dir: PathBuf,
    listen: String,
    device_name: String,
    memory_buffer: MemoryBuffer,
    tray_pipe: Option<String>,
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
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        DisableLineWrap,
        BeginSynchronizedUpdate,
    )?;

    // Windows 上把控制台缓冲区大小同步为窗口可见大小，避免滚动条导致
    // crossterm::terminal::size() 返回 buffer size 而非 window size。
    #[cfg(windows)]
    sync_console_buffer_size();

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 给 Windows Terminal / conhost 时间来完成 alternate screen 切换与尺寸同步。
    // run_app 中还会额外进行 2 次强制重绘，共同消除首帧布局错位。
    tokio::time::sleep(Duration::from_millis(100)).await;
    terminal.clear()?;

    let mut app = App::new(config_dir.clone(), listen, device_name, config);
    app.tray_pipe = tray_pipe;

    // 启动时检测是否已有外部 daemon（如 Auto 模式托盘启动的实例）
    if let Some(pid) = detect_external_daemon(&app.config, &config_dir).await {
        app.daemon_running = true;
        app.external_daemon = true;
        app.external_daemon_pid = Some(pid);

        // 若由托盘打开，尝试连接托盘 IPC，使 F5 能跨进程控制 daemon
        if let Some(pipe_name) = app.tray_pipe.clone() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                crate::tray_ipc::TrayIpcClient::connect(&pipe_name),
            )
            .await
            {
                Ok(Ok(mut client)) => {
                    // 发送 Ping 探测确认托盘存活
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

    // 恢复终端
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

    // daemon_join_handle: 后台 daemon 主循环的 JoinHandle，用于崩溃检测与优雅关闭等待
    let mut daemon_join_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut daemon_handle: Option<syncthing_net::ConnectionManagerHandle> = None;
    let mut event_tx: Option<tokio::sync::mpsc::Sender<TuiEvent>> = None;
    let mut daemon_shutdown_tx: Option<tokio::sync::watch::Sender<bool>> = None;

    // 首帧前等待终端尺寸稳定：Windows Terminal / cmd start 刚创建窗口时，
    // 控制台 srWindow 可能尚未同步到实际可见行数，导致首帧布局错位。
    // 轮询尺寸直到连续 10 次一致（约 500ms），最多等 3s，确保窗口管理器完成
    // 最终布局（如 configure_console_window 设置的 120x40 已经生效）。
    let mut stable_size = (0, 0);
    let mut stable_count = 0u8;
    for _i in 0..60 {
        // Windows 上优先用 WinAPI 读取真实可见窗口尺寸（srWindow），因为
        // crossterm::terminal::size() 在 ConPTY / Windows Terminal 首帧可能返回
        // 缓冲区大小或旧尺寸。
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

    // 尺寸稳定后、首帧绘制前强制同步并清屏：
    // 1. autoresize 让 ratatui 内部缓冲与后端一致；
    // 2. clear 消除 PowerShell / cmd 在 alternate screen 切换瞬间写入的残留内容；
    // 3. hide_cursor 避免初始光标闪烁；
    // 4. 立即 draw + flush，让用户看到的第一帧就是稳定画面。
    let _ = terminal.autoresize();
    let _ = terminal.clear();
    let _ = terminal.hide_cursor();
    let _ = terminal.draw(|f| ui::draw(f, app));
    let _ = terminal.flush();

    // 强制初始重绘：Windows Terminal / conhost 在 alternate screen 切换后，
    // 首帧后端缓冲区与实际显示可能尚未同步，导致底部状态栏与主内容区字符交错。
    // 连续再绘制 2 帧并 flush，给终端足够的时间完成尺寸同步，实测能消除
    // "初始布局混乱，resize 后正常" 的问题。
    for _n in 0..2 {
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = terminal.autoresize();
        let _ = terminal.clear();
        let _ = terminal.draw(|f| ui::draw(f, app));
        let _ = terminal.flush();
    }

    // 结束同步更新：此前 BeginSynchronizedUpdate 让终端延迟渲染，直到完整帧
    // 准备好。现在首帧已经稳定，允许终端正常刷新后续帧。
    // 直接对 stdout 下发命令，因为 run_app 的 backend 类型参数不一定实现 Write。
    let _ = execute!(io::stdout(), EndSynchronizedUpdate);

    // 终极兜底：首帧 layout 在部分 Windows 终端上无法通过纯软件重绘修复。
    // 利用"窗口 resize 后一定正常"的现象，在启动时主动触发一次真实的终端尺寸
    // 变化（先缩小 1 列再恢复），强制 ConPTY / Windows Terminal 重新分配可见
    // 缓冲区并完成一次真正的重排，从而消除首帧底行撕裂/错位。
    trigger_startup_resize().await;
    // resize 后必须再清屏并绘制一帧，让 ratatui 内部缓冲与新的终端状态同步。
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
                    // 窗口大小变化时同步 ratatui 内部尺寸并清屏，避免旧帧残留
                    // 与新的布局区域交错。
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
            // 优雅关闭 daemon（如果正在运行且由本 TUI 管理）
            if !app.external_daemon {
                if let Some(tx) = daemon_shutdown_tx.take() {
                    let _ = tx.send(true);
                    app.push_log("Daemon shutdown signal sent.".to_string());
                    // 让出一次时间片让 daemon 处理 shutdown 信号，避免阻塞 TUI 退出
                    tokio::task::yield_now().await;
                }
                // 等待 daemon 主循环在 2 秒内完成优雅关闭
                if let Some(handle) = daemon_join_handle.take() {
                    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
                }
                app.daemon_running = false;
                app.daemon_status = "Stopped".to_string();
            }
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
                        app.last_sync_progress_update = Some(Instant::now());
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            // P2: daemon 崩溃检测 —— 如果 daemon_running 但 handle 已失效或主循环已结束，标记为 Crashed
            // 外部 daemon 跳过此检测，因为它不由本 TUI 进程管理
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

            // 检测正在停止的 daemon 是否已完全退出，更新 UI 为 Stopped
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

            // 检测外部 daemon（托盘）进程是否已退出，若退出则将 TUI 切回本地管理。
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

            // 托盘托管模式：每 ~2 秒轮询一次 REST API，同步 daemon 实际状态
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

/// 使用 WinAPI 获取控制台真实可见窗口尺寸（srWindow）。
///
/// crossterm::terminal::size() 在 Windows Terminal / ConPTY 下有时会返回
/// 屏幕缓冲区大小或旧尺寸，导致 ratatui 首帧布局错位。`srWindow` 反映的是
/// 用户实际看到的窗口矩形，更可靠。
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

/// 将控制台缓冲区大小同步为当前窗口可见大小。
///
/// 这能避免缓冲区高度大于窗口高度时出现滚动条，进而避免 ratatui 把内容
/// 绘制到窗口底部之外的不可见区域。
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

/// 跨平台获取当前控制台可见尺寸。
///
/// Windows 上优先使用 WinAPI `srWindow`（真实窗口可见矩形），回退到
/// crossterm；其他平台直接使用 crossterm。
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

/// 在启动时主动触发一次真实的终端 resize。
///
/// 部分 Windows 终端（Windows Terminal / conhost + ConPTY）在 alternate screen
/// 切换后的首帧会出现底行布局错位，而用户手动 resize 后必然恢复正常。
/// 通过临时把终端尺寸缩小 1 列再立即恢复，可以欺骗终端完成一次真实的缓冲区
/// 重排，从而规避首帧渲染 bug。
async fn trigger_startup_resize() {
    use crossterm::terminal::SetSize;

    let Ok((w, h)) = crossterm::terminal::size() else {
        return;
    };
    if w < 2 || h < 2 {
        return;
    }
    // 先缩小 1 列；忽略错误（某些终端不支持 SetSize）。
    let _ = execute!(io::stdout(), SetSize(w - 1, h));
    // 极短延迟确保终端处理完尺寸变化事件。
    tokio::time::sleep(Duration::from_millis(50)).await;
    // 恢复原始尺寸。
    let _ = execute!(io::stdout(), SetSize(w, h));
    tokio::time::sleep(Duration::from_millis(50)).await;
}
