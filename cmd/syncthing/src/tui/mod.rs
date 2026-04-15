pub mod app;
pub mod daemon_runner;
pub mod events;
pub mod ui;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};
use tracing::warn;

use app::App;
use crate::logging_buffer::MemoryBuffer;

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

    let mut app = App::new(config_dir, listen, device_name, config);
    app.push_log("TUI started. Press F5 to run daemon.".to_string());

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

    let mut daemon_future: Option<std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>> = None;
    let mut daemon_handle: Option<syncthing_net::ConnectionManagerHandle> = None;

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

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
                    toggle_daemon(app, &mut daemon_future, &mut daemon_handle).await;
                    false
                }
                _ => events::handle_event(app, event),
            }
        } else {
            false
        };

        if should_exit {
            // 停止 daemon：取消 future
            daemon_future = None;
            daemon_handle = None;
            app.daemon_running = false;
            app.daemon_status = "Stopped".to_string();
            break;
        }

        if last_tick.elapsed() >= tick_rate {
            // 轮询 daemon 状态
            if let Some(ref handle) = daemon_handle {
                app.connected_devices = handle.connected_devices();
                if app.daemon_running {
                    app.daemon_status = format!("Running | {} devices connected", app.connected_devices.len());
                }
            }
            // 从内存日志缓冲区拉取新日志
            for line in memory_buffer.take_lines(100) {
                // 避免重复追加已经存在的日志行（简单去重：检查最后一条）
                if app.log_lines.back().map(|s| s.as_str()) != Some(line.as_str()) {
                    app.push_log(line);
                }
            }
            last_tick = tokio::time::Instant::now();
        }
    }

    Ok(())
}

async fn toggle_daemon(
    app: &mut App,
    daemon_future: &mut Option<std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>>,
    daemon_handle: &mut Option<syncthing_net::ConnectionManagerHandle>,
) {
    if daemon_future.is_some() {
        *daemon_future = None;
        *daemon_handle = None;
        app.daemon_running = false;
        app.daemon_status = "Stopped".to_string();
        app.push_log("Daemon stopped.".to_string());
    } else {
        let config_dir = app.config_dir.clone();
        let listen = app.listen.clone();
        let device_name = app.device_name.clone();

        match daemon_runner::start_daemon(config_dir, listen, device_name, false).await {
            Ok(startup) => {
                *daemon_handle = Some(startup.connection_handle);
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
