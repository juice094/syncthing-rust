#![windows_subsystem = "windows"]

use std::process::Stdio;

mod api;
mod tray;

use api::DaemonClient;
use tray::TrayEvent;

fn main() {
    // 1. 启动托盘线程（窗口 + 消息循环在独立线程）
    let event_rx = tray::spawn();

    // 2. 后台线程：tokio runtime + daemon 管理
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio");
        rt.block_on(daemon_loop());
    });

    // 3. 主线程：处理托盘事件
    for event in event_rx {
        match event {
            TrayEvent::OpenWebUi => open_web_ui(),
            TrayEvent::OpenTui => spawn_tui(),
            TrayEvent::ToggleDaemon => toggle_daemon(),
            TrayEvent::Exit => {
                stop_daemon();
                std::process::exit(0);
            }
        }
    }
}

// ==================== daemon management ====================

static DAEMON_LOCK: std::sync::Mutex<Option<tokio::process::Child>> = std::sync::Mutex::new(None);

fn config_dir() -> String {
    std::env::var("LOCALAPPDATA")
        .map(|p| format!("{}\\syncthing-rust", p))
        .unwrap_or_else(|_| ".".to_string())
}

fn syncthing_exe() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("syncthing.exe")
}

async fn daemon_loop() {
    let config_dir = config_dir();
    let mut client = DaemonClient::new("http://127.0.0.1:8385");
    let exe = syncthing_exe();
    let tray_hwnd = tray::tray_hwnd();

    // Auto-start daemon
    if !client.ping().await && exe.exists() {
        if let Ok(child) = tokio::process::Command::new(&exe)
            .arg("run")
            .arg("--config-dir")
            .arg(&config_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            for _ in 0..60 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                if client.ping().await {
                    break;
                }
            }
            *DAEMON_LOCK.lock().unwrap() = Some(child);
        }
    }

    // Status polling loop
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

        // Balloon notifications on state transitions
        unsafe {
            if s.online && !last_online {
                tray::show_notification(
                    tray_hwnd,
                    "Syncthing",
                    "Daemon is online",
                    tray::BalloonIcon::Info,
                );
            } else if !s.online && last_online {
                tray::show_notification(
                    tray_hwnd,
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
                tray::show_notification(tray_hwnd, "Syncthing", &msg, tray::BalloonIcon::Info);
            }

            if s.syncing && !last_syncing {
                tray::show_notification(
                    tray_hwnd,
                    "Syncthing",
                    "Sync started",
                    tray::BalloonIcon::Info,
                );
            }

            tray::update_tooltip(tray_hwnd, &tip);
            tray::update_icon(tray_hwnd, icon);
        }

        last_online = s.online;
        last_connected = s.connected_devices;
        last_syncing = s.syncing;
    }
}

fn spawn_tui() {
    let exe = syncthing_exe();
    let cd = config_dir();
    std::thread::spawn(move || {
        if !exe.exists() {
            tracing::error!("syncthing.exe not found at {}", exe.display());
            return;
        }
        // 使用 cmd.exe /c start 启动 TUI，确保新控制台窗口正确分配 std handles。
        // 直接 CreateProcess + CREATE_NEW_CONSOLE 会导致子进程继承父进程
        // （windows_subsystem，无控制台）的 INVALID_HANDLE_VALUE stdout，
        // crossterm::enable_raw_mode 因此失败。
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
            Ok(child) => tracing::info!("TUI spawned via cmd /c start: pid={}", child.id()),
            Err(e) => tracing::error!("Failed to spawn TUI: {}", e),
        }
    });
}

fn toggle_daemon() {
    let mut lock = DAEMON_LOCK.lock().unwrap();
    if let Some(mut child) = lock.take() {
        // Stop
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let _ = child.kill().await;
            });
        });
    } else {
        // Start
        let exe = syncthing_exe();
        let cd = config_dir();
        if exe.exists() {
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    if let Ok(child) = tokio::process::Command::new(&exe)
                        .arg("run")
                        .arg("--config-dir")
                        .arg(&cd)
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                    {
                        *DAEMON_LOCK.lock().unwrap() = Some(child);
                    }
                });
            });
        }
    }
}

fn stop_daemon() {
    if let Some(mut child) = DAEMON_LOCK.lock().unwrap().take() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = child.kill().await;
        });
    }
}

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
