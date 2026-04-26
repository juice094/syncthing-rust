use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use crate::handlers::validation;

use super::ApiState;
use super::parse_device_id;

pub(crate) async fn list_devices(State(state): State<ApiState>) -> impl IntoResponse {
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

pub(crate) async fn get_device(
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

pub(crate) async fn add_device(
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

pub(crate) async fn remove_device(
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

pub(crate) async fn update_device(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateDeviceRequest>,
) -> impl IntoResponse {
    let mut config = match state.config_store.load().await {
        Ok(c) => c,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    };

    let device_id = parse_device_id(&id);
    let request_id = parse_device_id(&request.id);

    if device_id != request_id {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("URL device ID '{}' does not match body device ID '{}'", id, request.id),
        ));
    }

    let device = match config.devices.iter_mut().find(|d| d.id.to_string() == device_id.to_string()) {
        Some(d) => d,
        None => return Err((StatusCode::NOT_FOUND, format!("Device '{}' not found", id))),
    };

    device.name = Some(request.name);
    device.addresses = request.addresses.into_iter().map(|a| match a.as_str() {
        "dynamic" => syncthing_core::types::AddressType::Dynamic,
        _ => syncthing_core::types::AddressType::Tcp(a),
    }).collect();
    device.introducer = request.introducer;

    let updated_device = device.clone();

    match state.config_store.save(&config).await {
        Ok(_) => Ok(Json(DeviceResponse::from(updated_device))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
    }
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
}

impl From<syncthing_core::types::Device> for DeviceResponse {
    fn from(device: syncthing_core::types::Device) -> Self {
        Self {
            id: device.id.to_string(),
            name: device.name.unwrap_or_default(),
            addresses: device.addresses.iter().map(|a| a.as_str().to_string()).collect(),
            introducer: device.introducer,
        }
    }
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
}
