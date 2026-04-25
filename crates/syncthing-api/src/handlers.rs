//! Module: syncthing-api
//! Worker: Agent-F
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证

//! HTTP request handlers for Syncthing API
//!
//! This module provides handler functions for processing HTTP requests,
//! including WebSocket upgrade handling for real-time events.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query,
        State,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use syncthing_core::types::Event;

use crate::rest::ApiState;

/// Generate a unique connection ID
fn generate_connection_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);

    format!("{}-{}", timestamp, counter)
}

/// WebSocket query parameters
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Event types to subscribe to (comma-separated)
    pub events: Option<String>,
    /// Since event ID (for catching up)
    pub since: Option<u64>,
    /// Limit number of events
    pub limit: Option<usize>,
}

/// WebSocket event message
#[derive(Debug, Clone, Serialize)]
pub struct EventMessage {
    /// Event ID (sequence number)
    pub id: u64,
    /// Event type
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event timestamp (Unix milliseconds)
    pub time: u64,
    /// Event data
    pub data: serde_json::Value,
}

/// Handle WebSocket upgrade for events endpoint
///
/// This handler upgrades HTTP connections to WebSocket connections
/// for receiving real-time events from Syncthing.
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<EventsQuery>,
    State(state): State<ApiState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_websocket(socket, state, query))
}

/// Handle WebSocket connection
async fn handle_websocket(socket: WebSocket, state: ApiState, query: EventsQuery) {
    let connection_id = format!("ws_{}", generate_connection_id());
    info!(
        "WebSocket connection established: {} (events: {:?}, since: {:?}, limit: {:?})",
        connection_id, query.events, query.since, query.limit
    );

    let event_filter: Option<Vec<String>> = query.events.as_ref().map(|e| {
        e.split(',').map(|s| s.trim().to_string()).collect()
    });

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Create channel for sending events to this connection
    let (event_tx, mut event_rx) = mpsc::channel::<Event>(100);

    // Register connection with event bus
    if let Err(e) = state.event_bus.register_connection(connection_id.clone(), event_tx).await {
        error!("Failed to register WebSocket connection: {}", e);
        return;
    }

    // Spawn task to forward events to WebSocket
    let forward_task = {
        let connection_id = connection_id.clone();
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let event_msg = convert_event_to_message(event);

                // Apply event type filter if specified
                if let Some(ref filter) = event_filter {
                    if !filter.contains(&event_msg.event_type) {
                        continue;
                    }
                }

                let json = match serde_json::to_string(&event_msg) {
                    Ok(j) => j,
                    Err(e) => {
                        error!("Failed to serialize event: {}", e);
                        continue;
                    }
                };

                if let Err(e) = ws_tx.send(Message::Text(json)).await {
                    debug!("WebSocket send error for {}: {}", connection_id, e);
                    break;
                }
            }
            debug!("Event forward task ended for {}", connection_id);
        })
    };

    // Handle incoming WebSocket messages
    while let Some(msg) = ws_rx.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                debug!("Received WebSocket message from {}: {}", connection_id, text);
                // Handle client messages (ping, subscription changes, etc.)
                handle_client_message(&connection_id, &text, &state).await;
            }
            Ok(Message::Binary(_)) => {
                debug!("Received binary message from {}", connection_id);
            }
            Ok(Message::Ping(data)) => {
                // Pong is handled automatically by axum
                debug!("Received ping from {}", connection_id);
                let _ = &data;
            }
            Ok(Message::Pong(_)) => {
                // Pong received, connection is alive
            }
            Ok(Message::Close(frame)) => {
                debug!("WebSocket closed by client {}: {:?}", connection_id, frame);
                break;
            }
            Err(e) => {
                error!("WebSocket error for {}: {}", connection_id, e);
                break;
            }
        }
    }

    // Cleanup
    info!("WebSocket connection closing: {}", connection_id);
    forward_task.abort();

    if let Err(e) = state.event_bus.unregister_connection(&connection_id).await {
        warn!("Failed to unregister WebSocket connection: {}", e);
    }
}

/// Handle a message from the WebSocket client
async fn handle_client_message(
    _connection_id: &str,
    text: &str,
    _state: &ApiState,
) {
    // Parse client message
    match serde_json::from_str::<ClientMessage>(text) {
        Ok(msg) => {
            match msg.action.as_str() {
                "ping" => {
                    // Client ping, server will respond with automatic pong
                    debug!("Received ping from client");
                }
                "subscribe" => {
                    debug!("Client subscription request: {:?}", msg.events);
                    // Subscription changes would be handled here
                }
                "unsubscribe" => {
                    debug!("Client unsubscription request: {:?}", msg.events);
                }
                _ => {
                    warn!("Unknown client action: {}", msg.action);
                }
            }
        }
        Err(e) => {
            debug!("Failed to parse client message: {}", e);
        }
    }
}

/// Client message format
#[derive(Debug, Deserialize)]
struct ClientMessage {
    /// Action to perform
    action: String,
    /// Event types to subscribe/unsubscribe
    events: Option<Vec<String>>,
}

/// Global event ID counter
static EVENT_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Convert internal Event to EventMessage for WebSocket
fn convert_event_to_message(event: Event) -> EventMessage {
    let event_type = match &event {
        Event::FolderSummary { .. } => "FolderSummary",
        Event::ItemFinished { .. } => "ItemFinished",
        Event::DeviceConnected { .. } => "DeviceConnected",
        Event::DeviceDisconnected { .. } => "DeviceDisconnected",
        Event::LocalIndexUpdated { .. } => "LocalIndexUpdated",
        Event::RemoteIndexUpdated { .. } => "RemoteIndexUpdated",
    };

    let data = match serde_json::to_value(&event) {
        Ok(mut value) => {
            // Remove the type field since we send it separately
            if let Some(obj) = value.as_object_mut() {
                obj.remove("type");
            }
            value
        }
        Err(e) => {
            error!("Failed to serialize event data: {}", e);
            serde_json::Value::Null
        }
    };

    EventMessage {
        id: EVENT_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        event_type: event_type.to_string(),
        time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        data,
    }
}

/// Error response for API errors
#[derive(Debug, Serialize)]
pub struct ApiError {
    /// Error code
    pub code: String,
    /// Error message
    pub message: String,
}

impl ApiError {
    /// Create a new API error
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Folder not found error
    pub fn folder_not_found(id: &str) -> Self {
        Self::new("FOLDER_NOT_FOUND", format!("Folder '{}' not found", id))
    }

    /// Device not found error
    pub fn device_not_found(id: &str) -> Self {
        Self::new("DEVICE_NOT_FOUND", format!("Device '{}' not found", id))
    }

    /// Invalid request error
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new("INVALID_REQUEST", message)
    }

    /// Internal server error
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new("INTERNAL_ERROR", message)
    }
}

/// API response wrapper
#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    /// Success flag
    pub success: bool,
    /// Response data (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Error information (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl<T: Serialize> ApiResponse<T> {
    /// Create a successful response
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    /// Create an error response
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(ApiError::new(code, message)),
        }
    }

    /// Create an error response from ApiError
    pub fn error_from(error: ApiError) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Pagination parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    /// Page number (1-based)
    #[serde(default = "default_page")]
    pub page: usize,
    /// Items per page
    #[serde(default = "default_per_page")]
    pub per_page: usize,
}

fn default_page() -> usize {
    1
}

fn default_per_page() -> usize {
    20
}

impl PaginationParams {
    /// Get offset for database queries
    pub fn offset(&self) -> usize {
        (self.page.saturating_sub(1)) * self.per_page
    }

    /// Get limit for database queries
    pub fn limit(&self) -> usize {
        self.per_page.clamp(1, 100) // Cap at 100 items per page
    }
}

/// Paginated response
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T> {
    /// Items for current page
    pub items: Vec<T>,
    /// Total number of items
    pub total: usize,
    /// Current page
    pub page: usize,
    /// Items per page
    pub per_page: usize,
    /// Total pages
    pub total_pages: usize,
}

impl<T> PaginatedResponse<T> {
    /// Create a new paginated response
    pub fn new(items: Vec<T>, total: usize, page: usize, per_page: usize) -> Self {
        let total_pages = (total + per_page - 1) / per_page.max(1);
        Self {
            items,
            total,
            page,
            per_page,
            total_pages,
        }
    }
}

/// Validation utilities
pub mod validation {
    use super::ApiError;

    fn map_err(e: syncthing_core::SyncthingError) -> ApiError {
        match e {
            syncthing_core::SyncthingError::Validation(msg) => ApiError::invalid_request(msg),
            _ => ApiError::invalid_request(e.to_string()),
        }
    }

    /// Validate folder ID
    pub fn validate_folder_id(id: &str) -> Result<(), ApiError> {
        syncthing_core::validation::validate_folder_id(id).map_err(map_err)
    }

    /// Validate device ID
    pub fn validate_device_id(id: &str) -> Result<(), ApiError> {
        syncthing_core::validation::validate_device_id(id).map_err(map_err)
    }

    /// Validate path
    pub fn validate_path(path: &str) -> Result<(), ApiError> {
        syncthing_core::validation::validate_path(path).map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::{FolderId, FolderSummary};

    #[test]
    fn test_convert_event_to_message() {
        let event = Event::FolderSummary {
            folder: FolderId::new("test"),
            summary: FolderSummary {
                files: 100,
                bytes: 1000,
                ..Default::default()
            },
        };

        let msg = convert_event_to_message(event);
        assert_eq!(msg.event_type, "FolderSummary");
        assert!(msg.data.get("folder").is_some());
    }

    #[test]
    fn test_api_response_success() {
        let response: ApiResponse<String> = ApiResponse::success("data".to_string());
        assert!(response.success);
        assert_eq!(response.data, Some("data".to_string()));
        assert!(response.error.is_none());
    }

    #[test]
    fn test_api_response_error() {
        let response: ApiResponse<String> = ApiResponse::error("ERROR", "Something went wrong");
        assert!(!response.success);
        assert!(response.data.is_none());
        assert!(response.error.is_some());
        let err = response.error.unwrap();
        assert_eq!(err.code, "ERROR");
        assert_eq!(err.message, "Something went wrong");
    }

    #[test]
    fn test_pagination_params() {
        let params = PaginationParams {
            page: 2,
            per_page: 20,
        };
        assert_eq!(params.offset(), 20);
        assert_eq!(params.limit(), 20);
    }

    #[test]
    fn test_paginated_response() {
        let items = vec!["a", "b", "c"];
        let response = PaginatedResponse::new(items, 100, 1, 20);
        assert_eq!(response.total, 100);
        assert_eq!(response.page, 1);
        assert_eq!(response.total_pages, 5);
    }

    #[test]
    fn test_validate_folder_id() {
        assert!(validation::validate_folder_id("valid-folder").is_ok());
        assert!(validation::validate_folder_id("").is_err());
        assert!(validation::validate_folder_id("invalid/folder").is_err());
        assert!(validation::validate_folder_id("a".repeat(65).as_str()).is_err());
    }

    #[test]
    fn test_validate_device_id() {
        // Valid device ID (Base32-Luhn format with dashes)
        let valid_id = "YTKWHNG-OT27ZGH-6VVBRIJ-OHOUNWT-DYLJ2NR-TCXUXHI-QDUQR2U-OPLCBQG";
        assert!(validation::validate_device_id(valid_id).is_ok());

        // Invalid - too short
        assert!(validation::validate_device_id("tooshort").is_err());

        // Invalid - empty
        assert!(validation::validate_device_id("").is_err());

        // Invalid - bad checksum (changed last char)
        let bad_checksum = "YTKWHNG-OT27ZGH-6VVBRIJ-OHOUNWT-DYLJ2NR-TCXUXHI-QDUQR2U-OPLCBQF";
        assert!(validation::validate_device_id(bad_checksum).is_err());
    }

    #[test]
    fn test_validate_path() {
        assert!(validation::validate_path("/valid/path").is_ok());
        assert!(validation::validate_path("").is_err());
        assert!(validation::validate_path("/path/../escape").is_err());
    }
}
