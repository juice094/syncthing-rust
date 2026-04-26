use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use tracing::warn;

use syncthing_core::types::FolderId;

use super::ApiState;
use super::parse_device_id;
use super::db::ErrorResponse;
use super::config::{update_config, UpdateConfigRequest};

pub(crate) async fn pause_all(State(state): State<ApiState>) -> impl IntoResponse {
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

pub(crate) async fn pause_folder(
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

pub(crate) async fn resume_all(State(state): State<ApiState>) -> impl IntoResponse {
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

pub(crate) async fn resume_folder(
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

pub(crate) async fn system_config_post(
    State(state): State<ApiState>,
    Json(request): Json<UpdateConfigRequest>,
) -> impl IntoResponse {
    update_config(State(state), Json(request)).await
}

pub(crate) async fn system_restart() -> impl IntoResponse {
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        std::process::exit(3);
    });
    Json(serde_json::json!({ "ok": "restarting" }))
}

pub(crate) async fn system_shutdown() -> impl IntoResponse {
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        std::process::exit(0);
    });
    Json(serde_json::json!({ "ok": "shutting down" }))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PauseResumeRequest {
    #[serde(default)]
    pub device: Vec<String>,
    #[serde(default)]
    pub folder: Vec<String>,
}

pub(crate) async fn system_pause(
    State(state): State<ApiState>,
    Json(request): Json<PauseResumeRequest>,
) -> impl IntoResponse {
    let mut paused = Vec::new();

    // Handle devices: update config and disconnect active connections
    if !request.device.is_empty() {
        match state.config_store.load().await {
            Ok(mut config) => {
                for device_id_str in &request.device {
                    let device_id = parse_device_id(device_id_str);
                    if let Some(device) = config.devices.iter_mut().find(|d| d.id.to_string() == device_id.to_string()) {
                        device.paused = true;
                        paused.push(device_id_str.clone());
                    }
                    if let Some(ref cm) = state.connection_manager {
                        if cm.has_connection(&device_id) {
                            if let Err(e) = cm.disconnect(&device_id, "paused by user").await {
                                warn!("Failed to disconnect device {}: {}", device_id_str, e);
                            }
                        }
                    }
                }
                if let Err(e) = state.config_store.save(&config).await {
                    warn!("Failed to save config after pause: {}", e);
                }
            }
            Err(e) => warn!("Failed to load config for pause: {}", e),
        }
    }

    if let Some(ref model) = state.sync_model {
        for folder_id in &request.folder {
            if let Err(e) = model.stop_folder(FolderId::new(folder_id)).await {
                warn!("Failed to pause folder {}: {}", folder_id, e);
            } else {
                paused.push(folder_id.clone());
            }
        }
    }
    Json(serde_json::json!({ "paused": paused }))
}

pub(crate) async fn system_resume(
    State(state): State<ApiState>,
    Json(request): Json<PauseResumeRequest>,
) -> impl IntoResponse {
    let mut resumed = Vec::new();

    // Handle devices: update config to un-pause
    if !request.device.is_empty() {
        match state.config_store.load().await {
            Ok(mut config) => {
                for device_id_str in &request.device {
                    let device_id = parse_device_id(device_id_str);
                    if let Some(device) = config.devices.iter_mut().find(|d| d.id.to_string() == device_id.to_string()) {
                        device.paused = false;
                        resumed.push(device_id_str.clone());
                    }
                }
                if let Err(e) = state.config_store.save(&config).await {
                    warn!("Failed to save config after resume: {}", e);
                }
            }
            Err(e) => warn!("Failed to load config for resume: {}", e),
        }
    }

    if let Some(ref model) = state.sync_model {
        for folder_id in &request.folder {
            if let Err(e) = model.start_folder(FolderId::new(folder_id)).await {
                warn!("Failed to resume folder {}: {}", folder_id, e);
            } else {
                resumed.push(folder_id.clone());
            }
        }
    }
    Json(serde_json::json!({ "resumed": resumed }))
}
