//! Prometheus metrics endpoint
//!
//! Manually emits Prometheus text format to avoid pulling in the `prometheus` crate.
//! Metrics are computed dynamically from ApiState on each scrape.

use axum::{extract::State, response::IntoResponse};
use std::fmt::Write;

use super::ApiState;

/// Prometheus text format Content-Type
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Handler for GET /metrics
pub(crate) async fn get_metrics(State(state): State<ApiState>) -> impl IntoResponse {
    let mut body = String::with_capacity(2048);

    // --- Build info ---
    writeln!(body, "# HELP syncthing_build_info Build information").ok();
    writeln!(body, "# TYPE syncthing_build_info gauge").ok();
    writeln!(
        body,
        r#"syncthing_build_info{{version="{}",os="{}",arch="{}"}} 1"#,
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
    .ok();
    writeln!(body).ok();

    // --- Uptime ---
    let uptime_secs = state.start_time.elapsed().as_secs_f64();
    writeln!(
        body,
        "# HELP syncthing_uptime_seconds Time since process start"
    )
    .ok();
    writeln!(body, "# TYPE syncthing_uptime_seconds gauge").ok();
    writeln!(body, "syncthing_uptime_seconds {:.3}", uptime_secs).ok();
    writeln!(body).ok();

    // --- Config snapshot ---
    let (device_count, folder_count) = if let Ok(config) = state.config_store.load().await {
        (config.devices.len(), config.folders.len())
    } else {
        (0, 0)
    };

    writeln!(
        body,
        "# HELP syncthing_configured_devices Number of configured devices"
    )
    .ok();
    writeln!(body, "# TYPE syncthing_configured_devices gauge").ok();
    writeln!(body, "syncthing_configured_devices {}", device_count).ok();
    writeln!(body).ok();

    writeln!(
        body,
        "# HELP syncthing_configured_folders Number of configured folders"
    )
    .ok();
    writeln!(body, "# TYPE syncthing_configured_folders gauge").ok();
    writeln!(body, "syncthing_configured_folders {}", folder_count).ok();
    writeln!(body).ok();

    // --- Connection metrics ---
    if let Some(ref cm) = state.connection_manager {
        let connected = cm.connected_devices();
        let stats = cm.connection_stats();

        writeln!(
            body,
            "# HELP syncthing_connected_devices Number of currently connected peers"
        )
        .ok();
        writeln!(body, "# TYPE syncthing_connected_devices gauge").ok();
        writeln!(body, "syncthing_connected_devices {}", connected.len()).ok();
        writeln!(body).ok();

        // Per-device connection state
        writeln!(
            body,
            "# HELP syncthing_device_connected Whether a device is currently connected (1=yes, 0=no)"
        )
        .ok();
        writeln!(body, "# TYPE syncthing_device_connected gauge").ok();
        for dev in &connected {
            let short = dev.to_string();
            writeln!(
                body,
                r#"syncthing_device_connected{{device_id="{}"}} 1"#,
                short
            )
            .ok();
        }
        writeln!(body).ok();

        writeln!(
            body,
            "# HELP syncthing_total_bytes_sent Total bytes sent across all BEP connections"
        )
        .ok();
        writeln!(body, "# TYPE syncthing_total_bytes_sent counter").ok();
        writeln!(
            body,
            "syncthing_total_bytes_sent {}",
            stats.total_bytes_sent
        )
        .ok();
        writeln!(body).ok();

        writeln!(
            body,
            "# HELP syncthing_total_bytes_received Total bytes received across all BEP connections"
        )
        .ok();
        writeln!(body, "# TYPE syncthing_total_bytes_received counter").ok();
        writeln!(
            body,
            "syncthing_total_bytes_received {}",
            stats.total_bytes_received
        )
        .ok();
        writeln!(body).ok();
    } else {
        writeln!(
            body,
            "# HELP syncthing_connected_devices Number of currently connected peers"
        )
        .ok();
        writeln!(body, "# TYPE syncthing_connected_devices gauge").ok();
        writeln!(body, "syncthing_connected_devices 0").ok();
        writeln!(body).ok();
    }

    // --- Folder file counts (via db if available) ---
    if let Some(ref db) = state.db {
        if let Ok(config) = state.config_store.load().await {
            writeln!(
                body,
                "# HELP syncthing_folder_files_total Number of files known in each folder"
            )
            .ok();
            writeln!(body, "# TYPE syncthing_folder_files_total gauge").ok();
            for folder in &config.folders {
                if let Ok(files) = db.get_folder_files(&folder.id).await {
                    writeln!(
                        body,
                        r#"syncthing_folder_files_total{{folder="{}"}} {}"#,
                        folder.id,
                        files.len()
                    )
                    .ok();
                }
            }
            writeln!(body).ok();
        }
    }

    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        body,
    )
}
