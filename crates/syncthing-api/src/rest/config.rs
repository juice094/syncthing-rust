use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use syncthing_core::types::Config;

use super::ApiState;
use super::{parse_device_id, parse_address};
use super::folder::FolderResponse;
use super::device::DeviceResponse;

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
    pub folders: Vec<super::folder::UpdateFolderRequest>,
    /// Updated devices
    pub devices: Vec<super::device::UpdateDeviceRequest>,
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

pub(crate) async fn get_config(State(state): State<ApiState>) -> impl IntoResponse {
    match state.config_store.load().await {
        Ok(config) => Ok(Json(ConfigResponse::from(config))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}

pub(crate) async fn update_config(
    State(state): State<ApiState>,
    Json(request): Json<UpdateConfigRequest>,
) -> impl IntoResponse {
    let mut config = match state.config_store.load().await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    };

    config.version = request.version as i32;

    // Merge folders by id
    for req_folder in request.folders {
        if let Some(ref id) = req_folder.id {
            if let Some(existing) = config.folders.iter_mut().find(|f| f.id == *id) {
                if let Some(label) = req_folder.label {
                    existing.label = Some(label);
                }
                if let Some(path) = req_folder.path {
                    existing.path = path;
                }
                if let Some(interval) = req_folder.rescan_interval_secs {
                    existing.rescan_interval_secs = interval as i32;
                }
            } else if let Some(path) = req_folder.path {
                let mut new_folder = syncthing_core::types::Folder::new(id.clone(), path);
                if let Some(label) = req_folder.label {
                    new_folder.label = Some(label);
                }
                if let Some(interval) = req_folder.rescan_interval_secs {
                    new_folder.rescan_interval_secs = interval as i32;
                }
                config.folders.push(new_folder);
            }
        }
    }

    // Merge devices by id
    for req_device in request.devices {
        let device_id = parse_device_id(&req_device.id);
        if let Some(existing) = config.devices.iter_mut().find(|d| d.id == device_id) {
            if !req_device.name.is_empty() {
                existing.name = Some(req_device.name);
            }
            if !req_device.addresses.is_empty() {
                existing.addresses = req_device.addresses.iter().map(parse_address).collect();
            }
            existing.introducer = req_device.introducer;
        } else {
            let device = syncthing_core::types::Device {
                id: device_id,
                name: if req_device.name.is_empty() { None } else { Some(req_device.name) },
                addresses: req_device.addresses.iter().map(parse_address).collect(),
                paused: false,
                introducer: req_device.introducer,
            };
            config.devices.push(device);
        }
    }

    match state.config_store.save(&config).await {
        Ok(_) => Ok(Json(ConfigResponse::from(config))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
}
