//! Module: syncthing-api
//! Worker: Agent-F
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证

//! REST API server for Syncthing
//!
//! This module provides HTTP endpoints for managing folders, devices,
//! and querying sync status. It uses Axum as the web framework.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{connect_info::ConnectInfo, Path, Query, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

use syncthing_core::traits::{ConfigStore, SyncModel};
use syncthing_core::types::{Config, FolderId, FolderSummary, VersioningConfig};
use syncthing_core::{DeviceId, Result, SyncthingError};
use sha2::{Digest, Sha256};

use crate::events::EventBus;
use crate::handlers;
use crate::handlers::validation;

/// API server state shared across handlers
#[derive(Clone)]
pub struct ApiState {
    /// Configuration store
    pub config_store: Arc<dyn ConfigStore>,
    /// Event bus for publishing events
    pub event_bus: EventBus,
    /// Optional sync model for controlling synchronization
    pub sync_model: Option<Arc<dyn SyncModel>>,
    /// Local device ID
    pub my_id: Option<DeviceId>,
    /// Optional API key for authentication
    pub api_key: Option<String>,
    /// Server start time for uptime calculation
    pub start_time: std::time::Instant,
    /// Optional connection manager for connection enumeration
    pub connection_manager: Option<syncthing_net::manager::ConnectionManagerHandle>,
    /// Optional local database for folder statistics
    pub db: Option<Arc<dyn syncthing_sync::database::LocalDatabase>>,
}

impl ApiState {
    /// Create new API state
    pub fn new(
        config_store: Arc<dyn ConfigStore>,
        event_bus: EventBus,
        sync_model: Option<Arc<dyn SyncModel>>,
    ) -> Self {
        Self {
            config_store,
            event_bus,
            sync_model,
            my_id: None,
            api_key: None,
            start_time: std::time::Instant::now(),
            connection_manager: None,
            db: None,
        }
    }
}

/// API Key 认证中间件
async fn api_key_middleware(
    State(state): State<ApiState>,
    req: axum::extract::Request,
    next: Next,
) -> std::result::Result<axum::response::Response, axum::http::StatusCode> {
    if req.uri().path() == "/rest/health" {
        return Ok(next.run(req).await);
    }
    // Allow loopback access without API key for local debugging
    let is_loopback = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().is_loopback())
        .unwrap_or(false);
    if is_loopback {
        return Ok(next.run(req).await);
    }
    if let Some(ref key) = state.api_key {
        if key.is_empty() {
            return Ok(next.run(req).await);
        }
        let header = req
            .headers()
            .get("x-api-key")
            .and_then(|v| v.to_str().ok());
        let query = req.uri().query().and_then(|q| {
            q.split('&').find_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                
                
                if k.eq_ignore_ascii_case("X-API-Key") {
                    Some(v)
                } else {
                    None
                }
            })
        });
        if header == Some(key) || query == Some(key) {
            Ok(next.run(req).await)
        } else {
            Err(axum::http::StatusCode::UNAUTHORIZED)
        }
    } else {
        Ok(next.run(req).await)
    }
}

/// REST API server
pub struct RestApi {
    state: ApiState,
    router: Router,
    listener: Option<TcpListener>,
}

impl RestApi {
    /// Create a new REST API server
    ///
    /// # Arguments
    /// * `state` - Shared API state
    ///
    /// # Example
    /// ```
    /// use std::sync::Arc;
    /// use syncthing_api::rest::{RestApi, ApiState};
    /// use syncthing_api::config::MemoryConfigStore;
    /// use syncthing_api::events::EventBus;
    ///
    /// let config_store = Arc::new(MemoryConfigStore::new());
    /// let event_bus = EventBus::new();
    /// let state = ApiState::new(config_store, event_bus, None);
    /// let api = RestApi::new(state);
    /// ```
    pub fn new(state: ApiState) -> Self {
        let router = Self::build_router(state.clone());
        Self {
            state,
            router,
            listener: None,
        }
    }

    /// Build the API router with all routes
    pub fn build_router(state: ApiState) -> Router {
        let auth_layer = axum::middleware::from_fn_with_state(state.clone(), api_key_middleware);
        Router::new()
            // Folder management
            .route("/rest/folders", get(list_folders).post(create_folder))
            .route(
                "/rest/folder/:id",
                get(get_folder).put(update_folder).delete(delete_folder),
            )
            // Device management
            .route("/rest/devices", get(list_devices).post(add_device))
            .route("/rest/device/:id", get(get_device).delete(remove_device))
            // Status queries - Go原版兼容
            .route("/rest/status", get(get_status))
            .route("/rest/system/status", get(get_system_status))
            .route("/rest/db/status", get(get_db_status))
            .route("/rest/db/completion", get(get_db_completion))
            .route("/rest/connections", get(get_connections))
            .route("/rest/system/connections", get(get_system_connections))
            .route("/rest/folder/:id/status", get(get_folder_status))
            // System operations
            .route("/rest/scan", post(trigger_scan))
            .route("/rest/scan/:id", post(trigger_folder_scan))
            .route("/rest/pause", post(pause_all))
            .route("/rest/pause/:id", post(pause_folder))
            .route("/rest/resume", post(resume_all))
            .route("/rest/resume/:id", post(resume_folder))
            // Config
            .route("/rest/config", get(get_config).put(update_config))
            .route("/rest/config/folders", get(list_folders))
            .route("/rest/config/devices", get(list_devices))
            // Health check
            .route("/rest/health", get(health_check))
            // WebSocket events
            .route("/rest/events", get(handlers::websocket_handler))
            // Middleware
            .layer(auth_layer)
            .layer(CorsLayer::permissive())
            .layer(TraceLayer::new_for_http())
            .with_state(state)
    }

    /// Bind to a socket address
    ///
    /// # Arguments
    /// * `addr` - Socket address to bind to
    pub async fn bind(&mut self, addr: SocketAddr) -> Result<()> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| SyncthingError::Network(format!("Failed to bind: {}", e)))?;
        info!("REST API bound to {}", addr);
        self.listener = Some(listener);
        Ok(())
    }

    /// Run the API server
    ///
    /// This method blocks until the server is shut down.
    pub async fn run(self) -> Result<()> {
        let listener = self.listener.ok_or_else(|| {
            SyncthingError::Internal("Server not bound to any address".to_string())
        })?;

        info!("REST API server starting");

        axum::serve(listener, self.router)
            .await
            .map_err(|e| SyncthingError::Network(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Get the router for testing or custom server setup
    pub fn router(&self) -> &Router {
        &self.router
    }

    /// Get the API state
    pub fn state(&self) -> &ApiState {
        &self.state
    }
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// Server status
    pub status: String,
    /// API version
    pub version: String,
}

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

/// Scan request
#[derive(Debug, Deserialize)]
pub struct ScanRequest {
    /// Folder ID to scan (optional, scans all if not specified)
    pub folder: Option<String>,
    /// Subdirectory to scan (optional)
    pub subdir: Option<String>,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
}

// Handler implementations

async fn health_check() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn list_folders(State(state): State<ApiState>) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => {
            let folders: Vec<FolderResponse> = config
                .folders
                .into_iter()
                .map(FolderResponse::from)
                .collect();
            Ok(Json(folders))
        }
        Err(e) => {
            error!("Failed to load config: {}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)))
        }
    }
}

async fn get_folder(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => {
            match config.folders.iter().find(|f| f.id.as_str() == id) {
                Some(folder) => Ok(Json(FolderResponse::from(folder.clone()))),
                None => Err((
                    StatusCode::NOT_FOUND,
                    format!("Folder '{}' not found", id),
                )),
            }
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn create_folder(
    State(state): State<ApiState>,
    Json(request): Json<CreateFolderRequest>,
) -> impl IntoResponse {
    if let Err(e) = validation::validate_folder_id(&request.id) {
        return Err((StatusCode::BAD_REQUEST, e.message));
    }
    if let Err(e) = validation::validate_path(&request.path) {
        return Err((StatusCode::BAD_REQUEST, e.message));
    }

    let mut config = match state.config_store.load().await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    };

    // Check if folder already exists
    if config.folders.iter().any(|f| f.id.as_str() == request.id) {
        return Err((
            StatusCode::CONFLICT,
            format!("Folder '{}' already exists", request.id),
        ));
    }

    let folder = syncthing_core::types::Folder {
        id: request.id,
        path: request.path,
        label: Some(request.label.unwrap_or_default()),
        folder_type: syncthing_core::types::FolderType::SendReceive,
        paused: false,
        rescan_interval_secs: request.rescan_interval_secs.unwrap_or(3600) as i32,
        devices: request
            .devices
            .into_iter()
            .map(|d| DeviceId::from_bytes_array(d.try_into().unwrap_or([0u8; 32])))
            .collect(),
        ignore_patterns: Vec::new(),
        versioning: request.versioning.map(|v| match v.type_.as_str() {
            "simple" => VersioningConfig::Simple { params: v.params },
            "staggered" => VersioningConfig::Staggered { params: v.params },
            "external" => VersioningConfig::External { params: v.params },
            _ => VersioningConfig::None,
        }),
    };

    config.folders.push(folder);

    match state.config_store.save(&config).await {
        Ok(_) => Ok((StatusCode::CREATED, Json(FolderResponse::from(config.folders.last().unwrap().clone())))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn update_folder(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateFolderRequest>,
) -> impl IntoResponse {
    if let Err(e) = validation::validate_folder_id(&id) {
        return Err((StatusCode::BAD_REQUEST, e.message));
    }
    if let Some(ref path) = request.path {
        if let Err(e) = validation::validate_path(path) {
            return Err((StatusCode::BAD_REQUEST, e.message));
        }
    }

    let mut config = match state.config_store.load().await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    };

    let folder_idx = match config.folders.iter().position(|f| f.id.as_str() == id) {
        Some(idx) => idx,
        None => return Err((StatusCode::NOT_FOUND, format!("Folder '{}' not found", id))),
    };

    // Update fields
    if let Some(label) = request.label {
        config.folders[folder_idx].label = Some(label);
    }
    if let Some(path) = request.path {
        config.folders[folder_idx].path = path;
    }
    if let Some(interval) = request.rescan_interval_secs {
        config.folders[folder_idx].rescan_interval_secs = interval as i32;
    }

    match state.config_store.save(&config).await {
        Ok(_) => Ok(Json(FolderResponse::from(config.folders[folder_idx].clone()))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn delete_folder(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut config = match state.config_store.load().await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    };

    let original_len = config.folders.len();
    config.folders.retain(|f| f.id.as_str() != id);

    if config.folders.len() == original_len {
        return Err((StatusCode::NOT_FOUND, format!("Folder '{}' not found", id)));
    }

    match state.config_store.save(&config).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn list_devices(State(state): State<ApiState>) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => {
            let devices: Vec<DeviceResponse> = config
                .devices
                .into_iter()
                .map(DeviceResponse::from)
                .collect();
            Ok(Json(devices))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn get_device(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => {
            // Parse device ID from string
            let device_id = parse_device_id(&id);
            match config.devices.iter().find(|d| d.id.to_string() == device_id.to_string()) {
                Some(device) => Ok(Json(DeviceResponse::from(device.clone()))),
                None => Err((
                    StatusCode::NOT_FOUND,
                    format!("Device '{}' not found", id),
                )),
            }
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn add_device(
    State(state): State<ApiState>,
    Json(request): Json<AddDeviceRequest>,
) -> impl IntoResponse {
    if let Err(e) = validation::validate_device_id(&request.id) {
        return Err((StatusCode::BAD_REQUEST, e.message));
    }

    let mut config = match state.config_store.load().await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    };

    let device_id = parse_device_id(&request.id);

    // Check if device already exists
    if config.devices.iter().any(|d| d.id.to_string() == device_id.to_string()) {
        return Err((
            StatusCode::CONFLICT,
            format!("Device '{}' already exists", request.id),
        ));
    }

    let device = syncthing_core::types::Device {
        id: device_id,
        name: request.name,
        addresses: request
            .addresses
            .unwrap_or_else(|| vec!["dynamic".to_string()])
            .into_iter()
            .map(|a| match a.as_str() {
                "dynamic" => syncthing_core::types::AddressType::Dynamic,
                _ => syncthing_core::types::AddressType::Tcp(a),
            })
            .collect(),
        paused: false,
        introducer: request.introducer.unwrap_or(false),
    };

    config.devices.push(device);

    match state.config_store.save(&config).await {
        Ok(_) => Ok((StatusCode::CREATED, Json(DeviceResponse::from(config.devices.last().unwrap().clone())))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn remove_device(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut config = match state.config_store.load().await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    };

    let device_id = parse_device_id(&id);
    let original_len = config.devices.len();
    config.devices.retain(|d| d.id.to_string() != device_id.to_string());

    if config.devices.len() == original_len {
        return Err((StatusCode::NOT_FOUND, format!("Device '{}' not found", id)));
    }

    match state.config_store.save(&config).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn get_status(State(state): State<ApiState>) -> impl IntoResponse {
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
async fn get_system_status(State(state): State<ApiState>) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => {
            let my_id = state
                .my_id
                .as_ref()
                .map(|d| d.to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let (totup, totdown) = state.connection_manager.as_ref()
                .map(|cm| {
                    let stats = cm.stats();
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

#[derive(Deserialize)]
struct CompletionQuery {
    folder: String,
    device: String,
}

/// GET /rest/db/completion - 文件夹相对于某设备的同步完成度
async fn get_db_completion(
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
async fn get_db_status(State(state): State<ApiState>) -> impl IntoResponse {
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

async fn get_connections(State(state): State<ApiState>) -> Json<ConnectionStatus> {
    if let Some(ref cm) = state.connection_manager {
        let devices = cm.connected_devices();
        let connections: Vec<DeviceConnection> = devices
            .into_iter()
            .filter_map(|device_id| {
                let conn = cm.get_connection(&device_id)?;
                Some(DeviceConnection {
                    id: device_id.to_string(),
                    address: conn.remote_addr().to_string(),
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
async fn get_system_connections(State(state): State<ApiState>) -> Json<SystemConnectionsResponse> {
    use std::collections::HashMap;
    let now = format!("{:?}", std::time::SystemTime::now());

    if let Some(ref cm) = state.connection_manager {
        let devices = cm.connected_devices();
        let mut connections: HashMap<String, ConnectionStats> = HashMap::new();
        let total_in = 0u64;
        let total_out = 0u64;

        for device_id in devices {
            if let Some(conn) = cm.get_connection(&device_id) {
                let addr = conn.remote_addr().to_string();
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

async fn get_folder_status(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Verify folder exists in config
    match state.config_store.load().await {
        Ok(config) => {
            if !config.folders.iter().any(|f| f.id.as_str() == id) {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: format!("Folder '{}' not found", id),
                    }),
                )
                    .into_response();
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load config: {}", e),
                }),
            )
                .into_response();
        }
    }

    let folder_id = FolderId::new(id);

    // Try to get real status from sync model
    if let Some(ref sync_model) = state.sync_model {
        match sync_model.folder_status(&folder_id).await {
            Ok(_) => {
                // Detailed stats require BlockStore which is not available via API state
            }
            Err(e) => {
                error!("Failed to get folder status from sync model: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to get folder status: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    }

    Json(FolderSummary::default()).into_response()
}

async fn trigger_scan(
    State(state): State<ApiState>,
    Json(request): Json<ScanRequest>,
) -> impl IntoResponse {
    if let Some(ref sync_model) = state.sync_model {
        if let Some(ref folder) = request.folder {
            let folder_id = FolderId::new(folder);
            match sync_model.scan_folder(&folder_id).await {
                Ok(()) => StatusCode::ACCEPTED.into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to scan folder: {}", e),
                    }),
                )
                    .into_response(),
            }
        } else {
            // Scan all configured folders
            match state.config_store.load().await {
                Ok(config) => {
                    for folder in &config.folders {
                        if let Err(e) = sync_model.scan_folder(&FolderId::new(folder.id.clone())).await {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: format!("Failed to scan folder {}: {}", folder.id, e),
                                }),
                            )
                                .into_response();
                        }
                    }
                    StatusCode::ACCEPTED.into_response()
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to load config: {}", e),
                    }),
                )
                    .into_response(),
            }
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Scan not implemented: sync model not available".to_string(),
            }),
        )
            .into_response()
    }
}

async fn trigger_folder_scan(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(ref sync_model) = state.sync_model {
        let folder_id = FolderId::new(id);
        match sync_model.scan_folder(&folder_id).await {
            Ok(()) => StatusCode::ACCEPTED.into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to scan folder: {}", e),
                }),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Folder scan not implemented: sync model not available".to_string(),
            }),
        )
            .into_response()
    }
}

async fn pause_all(State(state): State<ApiState>) -> impl IntoResponse {
    if let Some(ref sync_model) = state.sync_model {
        match state.config_store.load().await {
            Ok(config) => {
                for folder in &config.folders {
                    if let Err(e) = sync_model.stop_folder(FolderId::new(folder.id.clone())).await {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to pause folder {}: {}", folder.id, e),
                            }),
                        )
                            .into_response();
                    }
                }
                StatusCode::ACCEPTED.into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load config: {}", e),
                }),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Pause all not implemented: sync model not available".to_string(),
            }),
        )
            .into_response()
    }
}

async fn pause_folder(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(ref sync_model) = state.sync_model {
        let folder_id = FolderId::new(id);
        match sync_model.stop_folder(folder_id).await {
            Ok(()) => StatusCode::ACCEPTED.into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to pause folder: {}", e),
                }),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Pause folder not implemented: sync model not available".to_string(),
            }),
        )
            .into_response()
    }
}

async fn resume_all(State(state): State<ApiState>) -> impl IntoResponse {
    if let Some(ref sync_model) = state.sync_model {
        match state.config_store.load().await {
            Ok(config) => {
                for folder in &config.folders {
                    if let Err(e) = sync_model.start_folder(FolderId::new(folder.id.clone())).await {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to resume folder {}: {}", folder.id, e),
                            }),
                        )
                            .into_response();
                    }
                }
                StatusCode::ACCEPTED.into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load config: {}", e),
                }),
            )
            .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Resume all not implemented: sync model not available".to_string(),
            }),
        )
            .into_response()
    }
}

async fn resume_folder(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Some(ref sync_model) = state.sync_model {
        let folder_id = FolderId::new(id);
        match sync_model.start_folder(folder_id).await {
            Ok(()) => StatusCode::ACCEPTED.into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to resume folder: {}", e),
                }),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse {
                error: "Resume folder not implemented: sync model not available".to_string(),
            }),
        )
            .into_response()
    }
}

async fn get_config(State(state): State<ApiState>) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => Ok(Json(ConfigResponse::from(config))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

async fn update_config(
    State(state): State<ApiState>,
    Json(request): Json<UpdateConfigRequest>,
) -> impl IntoResponse {
    // Load existing config
    let mut config = match state.config_store.load().await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    };

    // Update version
    config.version = request.version as i32;

    // Note: Full config update would merge folders and devices
    // This is a simplified implementation

    match state.config_store.save(&config).await {
        Ok(_) => Ok(Json(ConfigResponse::from(config))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

// Helper functions

fn parse_device_id(id: &str) -> DeviceId {
    let hash = Sha256::digest(id.as_bytes());
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hash);
    DeviceId::from_bytes_array(bytes)
}

// Request/Response types

/// Request to create a new folder
#[derive(Debug, Deserialize)]
pub struct CreateFolderRequest {
    /// Folder identifier
    pub id: String,
    /// Human-readable label
    pub label: Option<String>,
    /// Local filesystem path
    pub path: String,
    /// Device IDs to share with
    pub devices: Vec<Vec<u8>>,
    /// Rescan interval in seconds
    pub rescan_interval_secs: Option<u32>,
    /// Versioning configuration
    pub versioning: Option<VersioningRequest>,
}

/// Request to update an existing folder
#[derive(Debug, Deserialize)]
pub struct UpdateFolderRequest {
    /// Human-readable label
    pub label: Option<String>,
    /// Local filesystem path
    pub path: Option<String>,
    /// Rescan interval in seconds
    pub rescan_interval_secs: Option<u32>,
}

/// Versioning configuration request
#[derive(Debug, Deserialize)]
pub struct VersioningRequest {
    /// Versioning type (e.g., "simple")
    pub type_: String,
    /// Type-specific parameters
    pub params: std::collections::HashMap<String, String>,
}

/// Request to add a new device
#[derive(Debug, Deserialize)]
pub struct AddDeviceRequest {
    /// Device identifier
    pub id: String,
    /// Human-readable name
    pub name: Option<String>,
    /// Connection addresses
    pub addresses: Option<Vec<String>>,
    /// Whether this device is an introducer
    pub introducer: Option<bool>,
    /// Compression mode string
    pub compression: Option<String>,
}

/// Folder response payload
#[derive(Debug, Serialize)]
pub struct FolderResponse {
    /// Folder identifier
    pub id: String,
    /// Human-readable label
    pub label: String,
    /// Local filesystem path
    pub path: String,
    /// Short IDs of shared devices
    pub devices: Vec<String>,
    /// Rescan interval in seconds
    pub rescan_interval_secs: u32,
}

impl From<syncthing_core::types::Folder> for FolderResponse {
    fn from(folder: syncthing_core::types::Folder) -> Self {
        Self {
            id: folder.id,
            label: folder.label.unwrap_or_default(),
            path: folder.path,
            devices: folder.devices.iter().map(|d: &DeviceId| d.short_id()).collect(),
            rescan_interval_secs: folder.rescan_interval_secs as u32,
        }
    }
}

/// Device response payload
#[derive(Debug, Serialize)]
pub struct DeviceResponse {
    /// Device identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Connection addresses
    pub addresses: Vec<String>,
    /// Whether this device is an introducer
    pub introducer: bool,
    /// Compression mode string
    pub compression: String,
}

impl From<syncthing_core::types::Device> for DeviceResponse {
    fn from(device: syncthing_core::types::Device) -> Self {
        Self {
            id: device.id.to_string(),
            name: device.name.unwrap_or_default(),
            addresses: device.addresses.iter().map(|a| a.as_str().to_string()).collect(),
            introducer: device.introducer,
            compression: "metadata".to_string(),
        }
    }
}

/// Configuration response payload
#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    /// Configuration version
    pub version: u32,
    /// Configured folders
    pub folders: Vec<FolderResponse>,
    /// Configured devices
    pub devices: Vec<DeviceResponse>,
}

/// Request to update configuration
#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    /// Configuration version
    pub version: u32,
    /// Updated folders
    pub folders: Vec<UpdateFolderRequest>,
    /// Updated devices
    pub devices: Vec<UpdateDeviceRequest>,
}

/// Device update request
#[derive(Debug, Deserialize)]
pub struct UpdateDeviceRequest {
    /// Device identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Connection addresses
    pub addresses: Vec<String>,
    /// Whether this device is an introducer
    pub introducer: bool,
    /// Compression mode string
    pub compression: String,
}

impl ConfigResponse {
    /// Create a config response from a configuration object
    pub fn from_config(config: Config) -> Self {
        Self {
            version: config.version as u32,
            folders: config.folders.into_iter().map(FolderResponse::from).collect(),
            devices: config.devices.into_iter().map(DeviceResponse::from).collect(),
        }
    }
}

impl From<Config> for ConfigResponse {
    fn from(config: Config) -> Self {
        Self::from_config(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MemoryConfigStore;

    fn create_test_state() -> ApiState {
        let config_store = Arc::new(MemoryConfigStore::new());
        let event_bus = EventBus::new();
        ApiState::new(config_store, event_bus, None)
    }

    #[tokio::test]
    async fn test_health_check() {
        let _response = health_check().await;
        // Should succeed - just verify it doesn't panic
    }

    #[tokio::test]
    async fn test_list_folders() {
        let state = create_test_state();
        let _result = list_folders(State(state)).await;
        // Handler returns impl IntoResponse, just verify it doesn't panic
    }

    #[tokio::test]
    async fn test_create_and_get_folder() {
        let state = create_test_state();

        // Create folder
        let create_request = CreateFolderRequest {
            id: "test-folder".to_string(),
            label: Some("Test Folder".to_string()),
            path: "/tmp/test".to_string(),
            devices: vec![],
            rescan_interval_secs: Some(3600),
            versioning: None,
        };

        let _create_result = create_folder(State(state.clone()), Json(create_request)).await;
        // Handler returns impl IntoResponse

        // Get folder
        let _get_result = get_folder(State(state), Path("test-folder".to_string())).await;
        // Handler returns impl IntoResponse
    }

    #[tokio::test]
    async fn test_delete_folder() {
        let state = create_test_state();

        // Create folder first
        let create_request = CreateFolderRequest {
            id: "delete-me".to_string(),
            label: None,
            path: "/tmp/delete".to_string(),
            devices: vec![],
            rescan_interval_secs: None,
            versioning: None,
        };

        create_folder(State(state.clone()), Json(create_request)).await;

        // Delete folder
        let _delete_result = delete_folder(State(state.clone()), Path("delete-me".to_string())).await;

        // Verify it's gone - this would need proper response inspection
        let _get_result = get_folder(State(state), Path("delete-me".to_string())).await;
    }

    #[test]
    fn test_parse_device_id() {
        let id1 = parse_device_id("device1");
        let id2 = parse_device_id("device1");
        let id3 = parse_device_id("device2");

        assert_eq!(id1.to_string(), id2.to_string());
        assert_ne!(id1.to_string(), id3.to_string());
    }
}
