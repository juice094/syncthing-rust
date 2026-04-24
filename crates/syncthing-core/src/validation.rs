//! 配置验证模块
//!
//! 提供设备、文件夹、路径等配置的统一验证逻辑，供 REST API 和 TUI 共享。

use crate::SyncthingError;

/// 验证文件夹 ID
pub fn validate_folder_id(id: &str) -> Result<(), SyncthingError> {
    if id.is_empty() {
        return Err(SyncthingError::Validation("Folder ID cannot be empty".to_string()));
    }
    if id.len() > 64 {
        return Err(SyncthingError::Validation(
            "Folder ID cannot exceed 64 characters".to_string(),
        ));
    }
    if !id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(SyncthingError::Validation(
            "Folder ID can only contain alphanumeric characters, hyphens, and underscores".to_string(),
        ));
    }
    Ok(())
}

/// 验证设备 ID 字符串格式
pub fn validate_device_id(id: &str) -> Result<(), SyncthingError> {
    if id.is_empty() {
        return Err(SyncthingError::Validation("Device ID cannot be empty".to_string()));
    }
    if let Err(e) = id.parse::<crate::DeviceId>() {
        return Err(SyncthingError::Validation(format!("Invalid device ID: {}", e)));
    }
    Ok(())
}

/// 验证路径字符串
pub fn validate_path(path: &str) -> Result<(), SyncthingError> {
    if path.is_empty() {
        return Err(SyncthingError::Validation("Path cannot be empty".to_string()));
    }
    if path.contains("..") {
        return Err(SyncthingError::Validation(
            "Path cannot contain parent directory references".to_string(),
        ));
    }
    Ok(())
}

/// 验证设备 ID 不是本机设备 ID
pub fn validate_device_id_not_local(id: &str, local: &crate::DeviceId) -> Result<(), SyncthingError> {
    if let Ok(device_id) = id.parse::<crate::DeviceId>() {
        if device_id == *local {
            return Err(SyncthingError::Validation(
                "Cannot add the local device as a remote device".to_string(),
            ));
        }
    }
    Ok(())
}

/// 验证文件夹路径不重复
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
