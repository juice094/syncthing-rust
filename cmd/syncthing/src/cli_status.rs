//! CLI `syncthing status` — 非交互式查询 daemon 运行状态

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

/// System status response (subset of /rest/system/status)
#[derive(Debug, Deserialize)]
struct SystemStatus {
    my_id: String,
    uptime: u64,
}

/// Connections response (subset of /rest/system/connections)
#[derive(Debug, Deserialize)]
struct Connections {
    #[serde(default)]
    connections: std::collections::HashMap<String, serde_json::Value>,
}

/// Query daemon status via REST API and print formatted output.
pub async fn run(config_path: &Path, json_output: bool) -> anyhow::Result<()> {
    let config = load_config(config_path)?;

    let api_addr = format!("http://{}", api_bind_to_localhost(&config.gui.address));
    let api_key = config.gui.api_key;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("build HTTP client")?;

    // Check if daemon is running via /rest/health
    let ping_url = format!("{}/rest/health", api_addr);
    match client.get(&ping_url).send().await {
        Ok(resp) if resp.status().is_success() => {}
        _ => {
            if json_output {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "not_running",
                        "message": "Daemon not running. Start with: syncthing run"
                    })
                );
            } else {
                println!("Status:     Not running");
                println!("修复:        syncthing run");
            }
            return Ok(());
        }
    }

    // Fetch system status
    let status_url = format!("{}/rest/system/status", api_addr);
    let system_status: SystemStatus = client
        .get(&status_url)
        .header("X-API-Key", &api_key)
        .send()
        .await
        .context("GET /rest/system/status")?
        .json()
        .await
        .context("parse system status")?;

    // Fetch connections
    let conn_url = format!("{}/rest/system/connections", api_addr);
    let connections: Connections = client
        .get(&conn_url)
        .header("X-API-Key", &api_key)
        .send()
        .await
        .context("GET /rest/system/connections")?
        .json()
        .await
        .context("parse connections")?;

    // Fetch folder count
    let folders_url = format!("{}/rest/config/folders", api_addr);
    let folders: Vec<serde_json::Value> = client
        .get(&folders_url)
        .header("X-API-Key", &api_key)
        .send()
        .await
        .context("GET /rest/config/folders")?
        .json()
        .await
        .context("parse folders")?;

    let connected = connections.connections.len();
    let total_devices = config.devices.len();
    let uptime_secs = system_status.uptime;

    if json_output {
        println!(
            "{}",
            serde_json::json!({
                "status": "running",
                "device_id": system_status.my_id,
                "uptime_seconds": uptime_secs,
                "connected_devices": connected,
                "total_devices": total_devices,
                "folder_count": folders.len(),
            })
        );
    } else {
        println!("Status:     Running");
        println!(
            "Uptime:     {}h {}m",
            uptime_secs / 3600,
            (uptime_secs % 3600) / 60
        );
        println!("Device ID:  {}", system_status.my_id);
        println!("Connected:  {}/{} devices", connected, total_devices);
        println!("Folders:    {}", folders.len());
    }

    Ok(())
}

fn load_config(path: &Path) -> anyhow::Result<syncthing_core::types::Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config from {:?}", path))?;
    let config: syncthing_core::types::Config = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse config from {:?}", path))?;
    Ok(config)
}

/// Extract port from bind address and replace host with localhost.
fn api_bind_to_localhost(addr: &str) -> String {
    let port = addr.rsplit(':').next().unwrap_or("8385");
    format!("127.0.0.1:{}", port)
}
