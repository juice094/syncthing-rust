//! Configuration validation.
//!
//! Shared validation logic for devices, folders, and paths, used by both the
//! REST API and the TUI.

use crate::SyncthingError;

/// Validates a folder ID.
///
/// # Errors
/// Returns `Validation` if the ID is empty, longer than 64 characters, or
/// contains characters other than alphanumerics, hyphens, and underscores.
pub fn validate_folder_id(id: &str) -> Result<(), SyncthingError> {
    if id.is_empty() {
        return Err(SyncthingError::Validation(
            "Folder ID cannot be empty".to_string(),
        ));
    }
    if id.len() > 64 {
        return Err(SyncthingError::Validation(
            "Folder ID cannot exceed 64 characters".to_string(),
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(SyncthingError::Validation(
            "Folder ID can only contain alphanumeric characters, hyphens, and underscores"
                .to_string(),
        ));
    }
    Ok(())
}

/// Validates a device ID string.
///
/// # Errors
/// Returns `Validation` if the string is empty or is not a parseable
/// [`crate::DeviceId`].
pub fn validate_device_id(id: &str) -> Result<(), SyncthingError> {
    if id.is_empty() {
        return Err(SyncthingError::Validation(
            "Device ID cannot be empty".to_string(),
        ));
    }
    if let Err(e) = id.parse::<crate::DeviceId>() {
        return Err(SyncthingError::Validation(format!(
            "Invalid device ID: {}",
            e
        )));
    }
    Ok(())
}

/// Validates a path string.
///
/// # Errors
/// Returns `Validation` if the path is empty or contains `..` (parent
/// directory references).
pub fn validate_path(path: &str) -> Result<(), SyncthingError> {
    if path.is_empty() {
        return Err(SyncthingError::Validation(
            "Path cannot be empty".to_string(),
        ));
    }
    if path.contains("..") {
        return Err(SyncthingError::Validation(
            "Path cannot contain parent directory references".to_string(),
        ));
    }
    Ok(())
}

/// Validates that a device ID string is not the local device ID.
///
/// # Errors
/// Returns `Validation` if the parsed ID equals `local`.
pub fn validate_device_id_not_local(
    id: &str,
    local: &crate::DeviceId,
) -> Result<(), SyncthingError> {
    if let Ok(device_id) = id.parse::<crate::DeviceId>() {
        if device_id == *local {
            return Err(SyncthingError::Validation(
                "Cannot add the local device as a remote device".to_string(),
            ));
        }
    }
    Ok(())
}

/// Validates that a folder path is not already used by another folder.
///
/// # Errors
/// Returns `Validation` naming the conflicting folder if `new_path` equals
/// the path of any folder except the one identified by `excluding_id`.
pub fn validate_no_duplicate_folder_paths(
    folders: &[crate::types::Folder],
    new_path: &str,
    excluding_id: Option<&str>,
) -> Result<(), SyncthingError> {
    for folder in folders {
        if Some(folder.id.as_str()) == excluding_id {
            continue;
        }
        if folder.path == new_path {
            return Err(SyncthingError::Validation(format!(
                "Path '{}' is already used by folder '{}'",
                new_path, folder.id
            )));
        }
    }
    Ok(())
}
