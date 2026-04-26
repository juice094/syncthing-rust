use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::error;

use syncthing_core::types::{FolderId, FolderSummary, VersioningConfig};
use syncthing_core::DeviceId;

use crate::handlers::validation;

use super::ApiState;
use super::db::ErrorResponse;

pub(crate) async fn list_folders(State(state): State<ApiState>) -> impl IntoResponse {
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

pub(crate) async fn get_folder(
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

pub(crate) async fn create_folder(
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

pub(crate) async fn update_folder(
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

pub(crate) async fn delete_folder(
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

pub(crate) async fn get_folder_status(
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
    /// Folder identifier (required when used inside UpdateConfigRequest)
    #[serde(default)]
    pub id: Option<String>,
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
