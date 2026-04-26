use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::error;

use syncthing_core::types::FolderId;
use syncthing_core::DeviceId;

use super::ApiState;

/// System status response - 兼容Go原版格式
#[derive(Debug, Serialize)]
pub struct SystemStatus {
    /// My device ID
    pub my_id: String,
    /// Server uptime in seconds
    pub uptime: u64,
    /// Number of configured folders
    pub folder_count: usize,
    /// Number of configured devices
    pub device_count: usize,
    /// Current version
    pub version: String,
    /// System architecture
    pub arch: String,
    /// Operating system
    pub os: String,
    /// Syncthing version
    pub syncthing_version: String,
    /// Total incoming traffic bytes
    pub totup: u64,
    /// Total outgoing traffic bytes
    pub totdown: u64,
}

/// Database status response - 兼容Go原版格式
#[derive(Debug, Serialize)]
pub struct DbStatus {
    /// Folder ID
    pub folder: String,
    /// Total files
    pub files: u64,
    /// Total directories
    pub directories: u64,
    /// Total symlinks
    pub symlinks: u64,
    /// Total bytes
    pub bytes: u64,
    /// Files needing sync
    pub need_files: u64,
    /// Directories needing sync
    pub need_directories: u64,
    /// Symlinks needing sync
    pub need_symlinks: u64,
    /// Bytes needing sync
    pub need_bytes: u64,
    /// Pull errors count
    pub pull_errors: u32,
    /// Sync percentage
    #[serde(rename = "globalBytes")]
    pub global_bytes: u64,
    /// Local bytes
    #[serde(rename = "localBytes")]
    pub local_bytes: u64,
    /// State (idle, scanning, syncing, error)
    pub state: String,
}

/// Connection status
#[derive(Debug, Serialize)]
pub struct ConnectionStatus {
    /// Total connections
    pub total: usize,
    /// Connected devices
    pub connections: Vec<DeviceConnection>,
}

/// Device connection info
#[derive(Debug, Serialize)]
pub struct DeviceConnection {
    /// Device ID
    pub id: String,
    /// Connection address
    pub address: String,
    /// Connection type
    pub conn_type: String,
    /// Connected since (Unix timestamp)
    pub connected_since: u64,
}

/// Go-compatible system connections response
#[derive(Debug, Serialize)]
pub struct SystemConnectionsResponse {
    /// Total stats
    pub total: ConnectionStats,
    /// Per-device connections (keyed by device ID)
    pub connections: std::collections::HashMap<String, ConnectionStats>,
}

/// Per-connection stats (Go-compatible)
#[derive(Debug, Serialize, Default)]
pub struct ConnectionStats {
    /// At timestamp
    pub at: String,
    /// Total bytes in
    #[serde(rename = "inBytesTotal")]
    pub in_bytes_total: u64,
    /// Total bytes out
    #[serde(rename = "outBytesTotal")]
    pub out_bytes_total: u64,
    /// Connection type
    #[serde(rename = "type")]
    pub conn_type: String,
    /// Whether connected
    pub connected: bool,
    /// Whether paused
    pub paused: bool,
    /// Address
    pub address: String,
}

#[derive(Deserialize)]
pub struct CompletionQuery {
    pub folder: String,
    pub device: String,
}

pub(crate) async fn get_status(State(state): State<ApiState>) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => {
            let status = SystemStatus {
                my_id: "UNKNOWN".to_string(), // Would come from certificate
                uptime: 0,                    // Would track actual uptime
                folder_count: config.folders.len(),
                device_count: config.devices.len(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                arch: std::env::consts::ARCH.to_string(),
                os: std::env::consts::OS.to_string(),
                syncthing_version: env!("CARGO_PKG_VERSION").to_string(),
                totup: 0,
                totdown: 0,
            };
            Ok(Json(status))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

/// GET /rest/system/status - 系统状态（Go原版格式）
pub(crate) async fn get_system_status(State(state): State<ApiState>) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => {
            let my_id = state
                .my_id
                .as_ref()
                .map(|d| d.to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let (totup, totdown) = state.connection_manager.as_ref()
                .map(|cm| {
                    let stats = cm.connection_stats();
                    (stats.total_bytes_sent, stats.total_bytes_received)
                })
                .unwrap_or((0, 0));

            let status = SystemStatus {
                my_id,
                uptime: state.start_time.elapsed().as_secs(),
                folder_count: config.folders.len(),
                device_count: config.devices.len(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                arch: std::env::consts::ARCH.to_string(),
                os: std::env::consts::OS.to_string(),
                syncthing_version: env!("CARGO_PKG_VERSION").to_string(),
                totup,
                totdown,
            };
            Ok(Json(status))
        }
        Err(e) => {
            error!("Failed to load config for system status: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)))
        }
    }
}

/// GET /rest/db/completion - 文件夹相对于某设备的同步完成度
pub(crate) async fn get_db_completion(
    State(state): State<ApiState>,
    Query(params): Query<CompletionQuery>,
) -> impl IntoResponse {
    let device_id = match params.device.parse::<DeviceId>() {
        Ok(id) => id,
        Err(e) => {
            return Err((StatusCode::BAD_REQUEST, format!("Invalid device ID: {}", e)));
        }
    };

    let folder_id = FolderId::new(params.folder);

    match state.sync_model {
        Some(ref sync_model) => {
            match sync_model.folder_completion(&folder_id, device_id).await {
                Ok(completion) => {
                    let resp = serde_json::json!({
                        "completion": completion,
                        "device": device_id.to_string(),
                        "folder": folder_id.as_str(),
                    });
                    Ok(Json(resp))
                }
                Err(e) => {
                    error!("Failed to get folder completion: {}", e);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)))
                }
            }
        }
        None => {
            let resp = serde_json::json!({
                "completion": 100,
                "device": device_id.to_string(),
                "folder": folder_id.as_str(),
            });
            Ok(Json(resp))
        }
    }
}

/// GET /rest/db/status - 数据库状态（Go原版格式）
pub(crate) async fn get_db_status(State(state): State<ApiState>) -> impl IntoResponse {
    // 返回所有文件夹的状态概览
    match state.config_store.load().await {
        Ok(config) => {
            let mut statuses = Vec::with_capacity(config.folders.len());
            
            for folder in &config.folders {
                let mut files_count = 0u64;
                let mut directories_count = 0u64;
                let mut symlinks_count = 0u64;
                let mut bytes_count = 0u64;
                
                // 从数据库获取真实文件统计
                if let Some(ref db) = state.db {
                    match db.get_folder_files(&folder.id).await {
                        Ok(file_infos) => {
                            for info in file_infos {
                                match info.file_type {
                                    syncthing_core::types::FileType::File => files_count += 1,
                                    syncthing_core::types::FileType::Directory => directories_count += 1,
                                    syncthing_core::types::FileType::Symlink => symlinks_count += 1,
                                }
                                bytes_count += info.size as u64;
                            }
                        }
                        Err(e) => {
                            error!("Failed to get folder files for {}: {}", folder.id, e);
                        }
                    }
                }
                
                // 从同步模型获取真实状态
                let folder_state = if let Some(ref sync_model) = state.sync_model {
                    match sync_model.folder_status(&FolderId::new(folder.id.clone())).await {
                        Ok(syncthing_core::traits::FolderStatus::Idle) => "idle".to_string(),
                        Ok(syncthing_core::traits::FolderStatus::Scanning) => "scanning".to_string(),
                        Ok(syncthing_core::traits::FolderStatus::Syncing { .. }) => "syncing".to_string(),
                        Ok(syncthing_core::traits::FolderStatus::Error { .. }) => "error".to_string(),
                        Ok(syncthing_core::traits::FolderStatus::Paused) => "paused".to_string(),
                        Err(e) => {
                            error!("Failed to get folder status for {}: {}", folder.id, e);
                            "unknown".to_string()
                        }
                    }
                } else {
                    "unknown".to_string()
                };
                
                statuses.push(DbStatus {
                    folder: folder.id.to_string(),
                    files: files_count,
                    directories: directories_count,
                    symlinks: symlinks_count,
                    bytes: bytes_count,
                    need_files: 0,
                    need_directories: 0,
                    need_symlinks: 0,
                    need_bytes: 0,
                    pull_errors: 0,
                    global_bytes: bytes_count,
                    local_bytes: bytes_count,
                    state: folder_state,
                });
            }
            
            Ok(Json(statuses))
        }
        Err(e) => {
            error!("Failed to load config for db status: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)))
        }
    }
}

pub(crate) async fn get_connections(State(state): State<ApiState>) -> Json<ConnectionStatus> {
    if let Some(ref cm) = state.connection_manager {
        let devices = cm.connected_devices();
        let connections: Vec<DeviceConnection> = devices
            .into_iter()
            .filter_map(|device_id| {
                let conn = cm.get_connection_info(&device_id)?;
                Some(DeviceConnection {
                    id: device_id.to_string(),
                    address: conn.remote_addr,
                    conn_type: "tcp-server".to_string(),
                    connected_since: 0,
                })
            })
            .collect();
        Json(ConnectionStatus {
            total: connections.len(),
            connections,
        })
    } else {
        Json(ConnectionStatus {
            total: 0,
            connections: vec![],
        })
    }
}

/// GET /rest/system/connections - Go-compatible connection enumeration
pub(crate) async fn get_system_connections(State(state): State<ApiState>) -> Json<SystemConnectionsResponse> {
    use std::collections::HashMap;
    let now = format!("{:?}", std::time::SystemTime::now());

    if let Some(ref cm) = state.connection_manager {
        let devices = cm.connected_devices();
        let mut connections: HashMap<String, ConnectionStats> = HashMap::new();
        let total_in = 0u64;
        let total_out = 0u64;

        for device_id in devices {
            if let Some(conn) = cm.get_connection_info(&device_id) {
                let addr = conn.remote_addr;
                connections.insert(
                    device_id.to_string(),
                    ConnectionStats {
                        at: now.clone(),
                        in_bytes_total: 0,
                        out_bytes_total: 0,
                        conn_type: "tcp-server".to_string(),
                        connected: true,
                        paused: false,
                        address: addr.clone(),
                    },
                );
            }
        }

        Json(SystemConnectionsResponse {
            total: ConnectionStats {
                at: now,
                in_bytes_total: total_in,
                out_bytes_total: total_out,
                conn_type: String::new(),
                connected: false,
                paused: false,
                address: String::new(),
            },
            connections,
        })
    } else {
        Json(SystemConnectionsResponse {
            total: ConnectionStats {
                at: now,
                in_bytes_total: 0,
                out_bytes_total: 0,
                conn_type: String::new(),
                connected: false,
                paused: false,
                address: String::new(),
            },
            connections: HashMap::new(),
        })
    }
}
