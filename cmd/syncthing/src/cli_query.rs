//! CLI 查询命令 — devices / folders / logs

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;
use serde_json::Value;

use crate::api_client::ApiClient;

fn build_client(config_path: &Path) -> anyhow::Result<ApiClient> {
    let config = crate::load_config(config_path)?;
    Ok(ApiClient::new(&config))
}

// ============================================================
// devices list
// ============================================================

#[derive(Debug, Deserialize)]
struct DeviceResponse {
    id: String,
    name: String,
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default)]
    paused: bool,
}

pub async fn devices_list(config_path: &Path) -> anyhow::Result<()> {
    let client = build_client(config_path)?;

    let devices: Vec<DeviceResponse> = client.get("/rest/config/devices").await?;
    let connections: Value = client.get("/rest/system/connections").await?;

    let connected_ids: std::collections::HashSet<String> = connections
        .get("connections")
        .and_then(|c| c.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    println!(
        "{:<20} {:<12} {:<8} ADDRESSES",
        "NAME", "DEVICE ID", "STATUS"
    );
    println!("{}", "-".repeat(70));
    for d in devices {
        let short_id = d.id.chars().take(7).collect::<String>();
        let status = if connected_ids.contains(&d.id) {
            "online"
        } else if d.paused {
            "paused"
        } else {
            "offline"
        };
        let addr = d.addresses.join(", ");
        println!("{:<20} {:<12} {:<8} {}", d.name, short_id, status, addr);
    }

    Ok(())
}

// ============================================================
// folders list
// ============================================================

#[derive(Debug, Deserialize)]
struct FolderResponse {
    id: String,
    path: String,
    #[serde(default)]
    devices: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FolderStatusResponse {
    #[serde(default)]
    files: u64,
    #[serde(default)]
    bytes: u64,
    #[serde(default)]
    need_files: u64,
    #[serde(default)]
    need_bytes: u64,
    #[serde(default)]
    pull_errors: u64,
}

pub async fn folders_list(config_path: &Path, with_status: bool) -> anyhow::Result<()> {
    let client = build_client(config_path)?;

    let folders: Vec<FolderResponse> = client.get("/rest/config/folders").await?;

    if with_status {
        println!(
            "{:<16} {:<10} {:<12} {:<10} PATH",
            "ID", "STATE", "DEVICES", "SIZE"
        );
        println!("{}", "-".repeat(80));
        for f in &folders {
            let status: FolderStatusResponse = client
                .get(&format!("/rest/folder/{}/status", f.id))
                .await
                .unwrap_or(FolderStatusResponse {
                    files: 0,
                    bytes: 0,
                    need_files: 0,
                    need_bytes: 0,
                    pull_errors: 0,
                });
            let state = if status.pull_errors > 0 {
                "error"
            } else if status.need_bytes > 0 {
                "syncing"
            } else {
                "idle"
            };
            let size = format_size(status.bytes);
            println!(
                "{:<16} {:<10} {:<12} {:<10} {}",
                f.id,
                state,
                f.devices.len(),
                size,
                f.path
            );
        }
    } else {
        println!("{:<16} {:<10} PATH", "ID", "DEVICES");
        println!("{}", "-".repeat(60));
        for f in &folders {
            println!("{:<16} {:<10} {}", f.id, f.devices.len(), f.path);
        }
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    format!("{:.1} {}", size, UNITS[unit_idx])
}

// ============================================================
// logs tail
// ============================================================

pub fn logs_tail(config_dir: &Path, tail: usize) -> anyhow::Result<()> {
    let logs_dir = config_dir.join("logs");
    if !logs_dir.exists() {
        anyhow::bail!(
            "Logs directory not found: {}\n修复: 先启动 daemon (syncthing run)",
            logs_dir.display()
        );
    }

    let mut entries: Vec<_> = std::fs::read_dir(&logs_dir)
        .with_context(|| format!("read logs dir {}", logs_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|e| {
        std::cmp::Reverse(
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        )
    });

    let latest = entries
        .first()
        .ok_or_else(|| anyhow::anyhow!("No log files found in {}", logs_dir.display()))?;

    let content = std::fs::read_to_string(latest.path())
        .with_context(|| format!("read log file {}", latest.path().display()))?;

    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(tail);
    for line in &lines[start..] {
        println!("{}", line);
    }

    Ok(())
}
