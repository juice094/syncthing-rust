//! CLI `syncthing status` — 非交互式查询 daemon 运行状态

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::api_client::ApiClient;

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
    let config = crate::load_config(config_path)?;
    let client = ApiClient::new(&config);

    // Check if daemon is running via /rest/health
    match client.get_raw("/rest/health").await {
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

    let system_status: SystemStatus = client
        .get("/rest/system/status")
        .await
        .context("GET /rest/system/status")?;
    let connections: Connections = client
        .get("/rest/system/connections")
        .await
        .context("GET /rest/system/connections")?;
    let folders: Vec<serde_json::Value> = client
        .get("/rest/config/folders")
        .await
        .context("GET /rest/config/folders")?;

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
