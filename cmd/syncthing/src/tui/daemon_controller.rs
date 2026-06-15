use std::path::Path;
use std::time::Duration;

use tracing::warn;

use super::app::App;
use super::constants;
use super::daemon_runner;
use super::TuiEvent;

/// 静默检查本地 REST API 是否可达。
pub async fn daemon_health_check(config: &syncthing_core::types::Config) -> bool {
    let api_port = config.gui.address.rsplit(':').next().unwrap_or("8385");
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let url = format!("http://127.0.0.1:{}/rest/health", api_port);
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// 检测是否已有外部 daemon 在运行。
///
/// 检查顺序：
/// 1. `config_dir/syncthing.pid` 是否存在且指向运行中的 syncthing-rust 进程
/// 2. ping 本地 REST API `/rest/health`（处理无 pid 文件但端口仍被占用的情况）
///
/// 返回 `Some(pid)` 表示检测到外部 daemon 并给出其 PID；
/// 返回 `None` 表示没有外部 daemon。
/// TUI 不应再启动内部实例，防止端口冲突和单实例锁问题。
pub async fn detect_external_daemon(
    config: &syncthing_core::types::Config,
    config_dir: &Path,
) -> Option<u32> {
    // 1. PID 文件证据链
    if let Some(pid) = crate::single_instance::running_instance_pid(config_dir) {
        tracing::info!(
            "External daemon detected via pid file: syncthing-rust is running (PID {})",
            pid
        );
        return Some(pid);
    }

    // 2. 端口/HTTP 证据链（兜底，清理过时的 pid 文件后仍有服务在监听时生效）
    let api_port = config.gui.address.rsplit(':').next().unwrap_or("8385");
    let url = format!("http://127.0.0.1:{}/rest/health", api_port);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return None,
    };

    match client.get(&url).send().await {
        Ok(resp) => {
            let external = resp.status().is_success();
            if external {
                tracing::info!(
                    "External daemon detected via HTTP ping on port {} (/{})",
                    api_port,
                    url
                );
                // HTTP 兜底无法得知真实 PID，用 0 占位。
                Some(0)
            } else {
                tracing::debug!(
                    "Port {} responded with status {}, no external daemon",
                    api_port,
                    resp.status()
                );
                None
            }
        }
        Err(e) => {
            tracing::debug!("No external daemon detected on port {}: {}", api_port, e);
            None
        }
    }
}

pub async fn toggle_daemon(
    app: &mut App,
    daemon_join_handle: &mut Option<tokio::task::JoinHandle<()>>,
    daemon_handle: &mut Option<syncthing_net::ConnectionManagerHandle>,
    event_tx: &mut Option<tokio::sync::mpsc::Sender<TuiEvent>>,
    daemon_shutdown_tx: &mut Option<tokio::sync::watch::Sender<bool>>,
) {
    // 托盘托管模式：通过 IPC 命令让托盘进程启停 daemon
    if app.external_daemon && app.tray_client.is_some() {
        let req = if app.daemon_running {
            crate::tray_ipc::TrayIpcRequest::StopDaemon
        } else {
            crate::tray_ipc::TrayIpcRequest::StartDaemon
        };
        if let Some(client) = app.tray_client.as_mut() {
            match client.send(req).await {
                Ok(resp) if resp.ok => {
                    app.daemon_status = if app.daemon_running {
                        "Stopping...".to_string()
                    } else {
                        "Starting...".to_string()
                    };
                    app.push_log(format!(
                        "Daemon {} command sent to tray.",
                        if app.daemon_running { "stop" } else { "start" }
                    ));
                }
                Ok(resp) => {
                    app.push_log(format!(
                        "Tray daemon command failed: {}",
                        resp.error.unwrap_or_default()
                    ));
                }
                Err(e) => {
                    app.push_log(format!("Failed to send command to tray: {}", e));
                }
            }
        }
        return;
    }

    // 外部 daemon 由托盘/Auto 模式管理，TUI 不能启动或停止它
    if app.external_daemon {
        app.push_log("Daemon is managed externally. Use the tray menu to stop it.".to_string());
        return;
    }

    // 使用 daemon_shutdown_tx 判断 daemon 是否运行（启动时设置，停止时 take）
    if daemon_shutdown_tx.is_some() {
        // 发送优雅关闭信号；不在这里同步等待，避免阻塞 TUI 事件循环
        if let Some(tx) = daemon_shutdown_tx.take() {
            let _ = tx.send(true);
            app.push_log("Daemon shutdown signal sent.".to_string());
        }
        *daemon_handle = None;
        *event_tx = None;
        app.sync_service = None;
        app.event_rx = None;
        app.daemon_running = false;
        app.daemon_status = "Stopping...".to_string();
        app.push_log("Daemon stopping...".to_string());
    } else {
        // 若上次停止的 daemon 仍在后台收尾，禁止立即重启，避免端口冲突
        if daemon_join_handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
        {
            app.popup = super::app::Popup::Error(
                "Daemon is still shutting down. Please wait a moment.".to_string(),
            );
            app.push_log(
                "Daemon start blocked: previous daemon is still shutting down.".to_string(),
            );
            return;
        }

        let config_dir = app.config_dir.clone();
        let listen = app.listen.clone();
        let device_name = app.device_name.clone();

        match daemon_runner::start_daemon(config_dir, listen, device_name).await {
            Ok(startup) => {
                *daemon_shutdown_tx = Some(startup.shutdown_tx.clone());
                *daemon_handle = Some(startup.connection_handle.clone());
                app.sync_service = Some(startup.sync_service.clone());

                // 启动事件桥
                let (tx, rx) = tokio::sync::mpsc::channel::<TuiEvent>(constants::EVENT_CHANNEL_CAP);
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
                let join_handle = tokio::spawn(async move {
                    if let Err(e) = fut.await {
                        warn!("Daemon exited with error: {}", e);
                    }
                });
                *daemon_join_handle = Some(join_handle);
                app.daemon_running = true;
                app.daemon_status = "Running".to_string();
                app.push_log("Daemon started.".to_string());
            }
            Err(e) => {
                app.popup = super::app::Popup::Error(format!("Failed to start daemon: {}", e));
                app.push_log(format!("Daemon start failed: {}", e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::Config;

    /// 启动一个最小 HTTP server，在 /rest/health 返回 200 OK。
    /// 仅用于测试 detect_external_daemon。
    async fn spawn_health_server(port: u16) -> tokio::task::JoinHandle<()> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("bind test health server");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept health request");
            let mut buf = [0u8; 1024];
            // 读取请求头（不解析，简单丢弃）
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
            let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;
        })
    }

    #[tokio::test]
    async fn test_detect_external_daemon_online() {
        let port = 18385u16;
        let handle = spawn_health_server(port).await;

        let mut config = Config::new();
        config.gui.address = format!("127.0.0.1:{}", port);

        let config_dir = std::env::temp_dir().join(format!(
            "syncthing-test-external-online-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&config_dir);
        std::fs::create_dir_all(&config_dir).unwrap();

        assert!(
            detect_external_daemon(&config, &config_dir).await.is_some(),
            "应检测到在线的外部 daemon"
        );

        let _ = std::fs::remove_dir_all(&config_dir);
        handle.abort();
    }

    #[tokio::test]
    async fn test_detect_external_daemon_offline() {
        let mut config = Config::new();
        config.gui.address = "127.0.0.1:18386".to_string();

        let config_dir = std::env::temp_dir().join(format!(
            "syncthing-test-external-offline-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&config_dir);
        std::fs::create_dir_all(&config_dir).unwrap();

        assert!(
            detect_external_daemon(&config, &config_dir).await.is_none(),
            "未运行的 daemon 应返回 None"
        );

        let _ = std::fs::remove_dir_all(&config_dir);
    }
}
