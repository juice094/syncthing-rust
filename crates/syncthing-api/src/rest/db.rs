use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use syncthing_core::types::FolderId;

use super::ApiState;

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

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DbScanRequest {
    pub folder: String,
    #[serde(default)]
    pub sub: Option<String>,
}

pub(crate) async fn trigger_scan(
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

pub(crate) async fn trigger_folder_scan(
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

pub(crate) async fn db_scan_post(
    State(state): State<ApiState>,
    Json(request): Json<DbScanRequest>,
) -> impl IntoResponse {
    if let Some(ref model) = state.sync_model {
        let result = match request.sub {
            Some(ref sub) if !sub.is_empty() => {
                model.scan_folder_sub(&FolderId::new(&request.folder), sub).await
            }
            _ => model.scan_folder(&FolderId::new(&request.folder)).await,
        };
        match result {
            Ok(_) => Ok(Json(serde_json::json!({ "ok": true }))),
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("{}", e) })),
            )),
        }
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "sync model not available" })),
        ))
    }
}

pub(crate) async fn db_override() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({ "error": "override not yet implemented" })),
    )
        .into_response()
}

pub(crate) async fn db_revert() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({ "error": "revert not yet implemented" })),
    )
        .into_response()
}
