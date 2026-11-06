use std::path::Path;
use std::time::Duration;

use tracing::warn;

use super::app::App;
use super::constants;
use super::daemon_runner;
use super::TuiEvent;

/// Probes the local REST API without side effects.
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

/// Detects whether an external daemon is already running.
///
/// Evidence chain:
/// 1. a live `config_dir/syncthing.pid` pointing at a running syncthing-rust process;
/// 2. a successful ping of the local REST API `/rest/health` (covers the case
///    where the pid file was cleaned up but the service still holds the port).
///
/// Returns `Some(pid)` when an external daemon was detected (`0` when only the
/// HTTP evidence is available), `None` otherwise. When detected, the TUI must
/// not start its own instance (port conflicts and the single-instance lock).
pub async fn detect_external_daemon(
    config: &syncthing_core::types::Config,
    config_dir: &Path,
) -> Option<u32> {
    if let Some(pid) = crate::single_instance::running_instance_pid(config_dir) {
        tracing::info!(
            "External daemon detected via pid file: syncthing-rust is running (PID {})",
            pid
        );
        return Some(pid);
    }

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
                // The HTTP fallback cannot recover the real PID; 0 is a placeholder.
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

/// Toggles the daemon on F5.
///
/// In tray-managed mode the command is forwarded over tray IPC; for external
/// daemons the toggle is refused (they belong to the tray / Auto-mode process).
/// Otherwise the locally managed daemon is started or signalled to stop.
pub async fn toggle_daemon(
    app: &mut App,
    daemon_join_handle: &mut Option<tokio::task::JoinHandle<()>>,
    daemon_handle: &mut Option<syncthing_net::ConnectionManagerHandle>,
    event_tx: &mut Option<tokio::sync::mpsc::Sender<TuiEvent>>,
    daemon_shutdown_tx: &mut Option<tokio::sync::watch::Sender<bool>>,
) {
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

    // External daemons belong to the tray / Auto-mode process; the TUI cannot start or stop them.
    if app.external_daemon {
        app.push_log("Daemon is managed externally. Use the tray menu to stop it.".to_string());
        return;
    }

    // daemon_shutdown_tx presence means "running" (set at start, taken at stop).
    if daemon_shutdown_tx.is_some() {
        // Signal a graceful shutdown without waiting here, so the TUI event loop is not blocked.
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
        // Refuse to restart while the previous daemon is still tearing down (port conflicts).
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
