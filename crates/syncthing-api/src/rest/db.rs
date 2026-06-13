use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use syncthing_core::types::{FileInfo, FileType, FolderId};

use super::ApiState;

/// Scan request
#[derive(Debug, Deserialize)]
pub struct ScanRequest {
    /// Folder ID to scan (optional, scans all if not specified)
    pub folder: Option<String>,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
}

#[derive(Debug, Deserialize)]
// NOTE: Fields accessed via serde deserialization, not direct construction
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
                        if let Err(e) = sync_model
                            .scan_folder(&FolderId::new(folder.id.clone()))
                            .await
                        {
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
                model
                    .scan_folder_sub(&FolderId::new(&request.folder), sub)
                    .await
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

#[derive(Debug, Deserialize)]
pub struct DbOverrideRequest {
    pub folder: String,
}

#[derive(Debug, Deserialize)]
pub struct DbRevertRequest {
    pub folder: String,
}

pub(crate) async fn db_override(
    State(state): State<ApiState>,
    Json(request): Json<DbOverrideRequest>,
) -> impl IntoResponse {
    if let Some(ref sync_model) = state.sync_model {
        match sync_model
            .override_folder(&FolderId::new(&request.folder))
            .await
        {
            Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
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

pub(crate) async fn db_revert(
    State(state): State<ApiState>,
    Json(request): Json<DbRevertRequest>,
) -> impl IntoResponse {
    if let Some(ref sync_model) = state.sync_model {
        match sync_model
            .revert_folder(&FolderId::new(&request.folder))
            .await
        {
            Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
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

// ============================================
// Batch A1: db/browse
// ============================================

/// Browse query parameters
#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    /// Folder ID to browse
    pub folder: String,
    /// Prefix path filter (optional)
    pub prefix: Option<String>,
    /// Max depth levels (default: unlimited)
    pub levels: Option<usize>,
}

/// Single browse entry (file or directory)
#[derive(Debug, Serialize)]
pub struct BrowseEntry {
    /// File or directory name (last path segment)
    pub name: String,
    /// Entry type: "file" | "directory" | "symlink"
    #[serde(rename = "type")]
    pub entry_type: String,
    /// File size in bytes (only for files)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    /// Modification time in seconds (only for files)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_s: Option<i64>,
    /// Child entries (only for directories)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<BrowseEntry>,
}

/// Build a tree from flat file list.
///
/// Algorithm: group by first path segment, recurse for subdirectories.
fn build_tree(
    files: &[FileInfo],
    prefix: &str,
    levels: usize,
    current_depth: usize,
) -> Vec<BrowseEntry> {
    if levels > 0 && current_depth >= levels {
        return Vec::new();
    }

    // Collect immediate children under the current prefix
    let mut entries: std::collections::BTreeMap<String, (FileInfo, Vec<FileInfo>)> =
        std::collections::BTreeMap::new();
    let mut direct_files: Vec<FileInfo> = Vec::new();

    for file in files {
        let relative = if prefix.is_empty() {
            file.name.clone()
        } else if let Some(stripped) = file.name.strip_prefix(prefix) {
            // Remove leading slash if present
            stripped.strip_prefix('/').unwrap_or(stripped).to_string()
        } else {
            continue;
        };

        if relative.is_empty() {
            // The prefix itself is a file (or exact match)
            direct_files.push(file.clone());
            continue;
        }

        let first_sep = relative.find('/');
        let first_segment = match first_sep {
            Some(idx) => &relative[..idx],
            None => relative.as_str(),
        };

        if first_sep.is_none() {
            // Direct child file
            direct_files.push(file.clone());
        } else {
            // Subdirectory entry — collect all files under this segment
            let sub_prefix = if prefix.is_empty() {
                first_segment.to_string()
            } else {
                format!("{}/{}", prefix, first_segment)
            };

            entries
                .entry(first_segment.to_string())
                .or_insert_with(|| {
                    (
                        FileInfo {
                            name: sub_prefix.clone(),
                            file_type: FileType::Directory,
                            size: 0,
                            permissions: 0,
                            modified_s: 0,
                            modified_ns: 0,
                            version: Default::default(),
                            sequence: 0,
                            block_size: 0,
                            blocks: Vec::new(),
                            symlink_target: None,
                            deleted: Some(false),
                            modified_by: None,
                            blocks_hash: None,
                            no_permissions: None,
                            base_version: None,
                        },
                        Vec::new(),
                    )
                })
                .1
                .push(file.clone());
        }
    }

    let mut result: Vec<BrowseEntry> = Vec::new();

    // Add directory entries first
    for (name, (_dir_info, sub_files)) in entries {
        let sub_prefix = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        let children = build_tree(&sub_files, &sub_prefix, levels, current_depth + 1);
        result.push(BrowseEntry {
            name,
            entry_type: "directory".to_string(),
            size: None,
            modified_s: None,
            children,
        });
    }

    // Add direct file entries
    for file in direct_files {
        let name = if prefix.is_empty() {
            file.name.clone()
        } else if let Some(stripped) = file.name.strip_prefix(prefix) {
            stripped.strip_prefix('/').unwrap_or(stripped).to_string()
        } else {
            file.name.clone()
        };
        // Only include if name doesn't contain '/' (i.e., it's a direct child)
        if !name.contains('/') {
            result.push(BrowseEntry {
                name,
                entry_type: match file.file_type {
                    FileType::File => "file".to_string(),
                    FileType::Directory => "directory".to_string(),
                    FileType::Symlink => "symlink".to_string(),
                },
                size: if file.file_type == FileType::File {
                    Some(file.size)
                } else {
                    None
                },
                modified_s: if file.file_type == FileType::File {
                    Some(file.modified_s)
                } else {
                    None
                },
                children: Vec::new(),
            });
        }
    }

    result
}

pub(crate) async fn browse(
    State(state): State<ApiState>,
    Query(query): Query<BrowseQuery>,
) -> impl IntoResponse {
    let db = match state.db {
        Some(ref db) => db,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Database not available".to_string(),
            ));
        }
    };

    // Verify folder exists in config
    match state.config_store.load().await {
        Ok(config) => {
            if !config.folders.iter().any(|f| f.id == query.folder) {
                return Err((
                    StatusCode::NOT_FOUND,
                    format!("Folder '{}' not found", query.folder),
                ));
            }
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load config: {}", e),
            ));
        }
    }

    let files = match db.get_folder_files(&query.folder).await {
        Ok(files) => files,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read folder files: {}", e),
            ));
        }
    };

    let prefix = query.prefix.unwrap_or_default();
    let levels = query.levels.unwrap_or(0);

    let tree = build_tree(&files, &prefix, levels, 0);
    Ok(Json(tree))
}

// ============================================
// Batch A2: db/file
// ============================================

/// File query parameters
#[derive(Debug, Deserialize)]
pub struct FileQuery {
    /// Folder ID
    pub folder: String,
    /// Relative file path within the folder
    pub file: String,
}

/// File info response
#[derive(Debug, Serialize)]
pub struct FileInfoResponse {
    /// File name (relative path)
    pub name: String,
    /// File type
    #[serde(rename = "type")]
    pub file_type: String,
    /// File size in bytes
    pub size: i64,
    /// Modification time (seconds)
    pub modified_s: i64,
    /// Modification time (nanoseconds)
    pub modified_ns: i32,
    /// File permissions
    pub permissions: u32,
    /// Whether the file is deleted
    pub deleted: bool,
    /// Number of blocks
    pub num_blocks: usize,
}

impl From<FileInfo> for FileInfoResponse {
    fn from(info: FileInfo) -> Self {
        let deleted = info.is_deleted();
        Self {
            name: info.name,
            file_type: match info.file_type {
                FileType::File => "file".to_string(),
                FileType::Directory => "directory".to_string(),
                FileType::Symlink => "symlink".to_string(),
            },
            size: info.size,
            modified_s: info.modified_s,
            modified_ns: info.modified_ns,
            permissions: info.permissions,
            deleted,
            num_blocks: info.blocks.len(),
        }
    }
}

pub(crate) async fn file_info(
    State(state): State<ApiState>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let db = match state.db {
        Some(ref db) => db,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Database not available".to_string(),
            ));
        }
    };

    // Verify folder exists
    match state.config_store.load().await {
        Ok(config) => {
            if !config.folders.iter().any(|f| f.id == query.folder) {
                return Err((
                    StatusCode::NOT_FOUND,
                    format!("Folder '{}' not found", query.folder),
                ));
            }
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load config: {}", e),
            ));
        }
    }

    let files = match db.get_folder_files(&query.folder).await {
        Ok(files) => files,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read folder files: {}", e),
            ));
        }
    };

    match files.into_iter().find(|f| f.name == query.file) {
        Some(info) => Ok(Json(FileInfoResponse::from(info))),
        None => Err((
            StatusCode::NOT_FOUND,
            format!(
                "File '{}' not found in folder '{}'",
                query.file, query.folder
            ),
        )),
    }
}
