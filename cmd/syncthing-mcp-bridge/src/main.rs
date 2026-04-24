//! syncthing-mcp-bridge
//!
//! MCP (Model Context Protocol) Bridge for syncthing-rust.
//! Translates MCP stdio JSON-RPC messages into REST API calls.
//!
//! Architecture: Kimi/Claude ↔ MCP stdio ↔ this bridge ↔ HTTP REST ↔ syncthing-rust

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;

// ─── JSON-RPC 2.0 Types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcError {
    fn parse_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32700,
            message: msg.into(),
        }
    }
    fn method_not_found() -> Self {
        Self {
            code: -32601,
            message: "Method not found".into(),
        }
    }
    fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
        }
    }
    fn internal_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: msg.into(),
        }
    }
}

// ─── MCP Server ───────────────────────────────────────────────────────────

struct McpServer {
    client: reqwest::Client,
    base_url: String,
}

impl McpServer {
    fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
        }
    }

    async fn handle(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        // Validate JSON-RPC version
        if req.jsonrpc != "2.0" {
            return Some(JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: None,
                error: Some(JsonRpcError::parse_error("Invalid jsonrpc version")),
            });
        }
        // Notifications have no id or id is null — no response needed
        let is_notification =
            req.id.is_none() || req.id.as_ref().is_some_and(|v| v.is_null());

        let result = match req.method.as_str() {
            "initialize" => self.handle_initialize(req.params).await,
            "notifications/initialized" => {
                tracing::debug!("Client initialized notification received");
                Ok(Value::Null)
            }
            "tools/list" => self.handle_tools_list().await,
            "tools/call" => self.handle_tools_call(req.params).await,
            "resources/list" => self.handle_resources_list().await,
            "resources/read" => self.handle_resources_read(req.params).await,
            _ => Err(JsonRpcError::method_not_found()),
        };

        if is_notification {
            return None;
        }

        let (result, error) = match result {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };

        Some(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id,
            result,
            error,
        })
    }

    // ─── Initialize ───────────────────────────────────────────────────────

    async fn handle_initialize(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        Ok(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {"listChanged": false},
                "resources": {"subscribe": false, "listChanged": false}
            },
            "serverInfo": {
                "name": "syncthing-mcp-bridge",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    }

    // ─── Tools ────────────────────────────────────────────────────────────

    async fn handle_tools_list(&self) -> Result<Value, JsonRpcError> {
        Ok(serde_json::json!({
            "tools": [
                {
                    "name": "list_devices",
                    "description": "List all configured devices in syncthing",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "get_device",
                    "description": "Get details of a specific device",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "device_id": {"type": "string", "description": "Device ID string"}
                        },
                        "required": ["device_id"]
                    }
                },
                {
                    "name": "add_device",
                    "description": "Add a new remote device",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "device_id": {"type": "string", "description": "Device ID (e.g., XQVFE6J-...)"},
                            "name": {"type": "string", "description": "Display name (optional)"},
                            "addresses": {"type": "array", "items": {"type": "string"}, "description": "Connection addresses (optional, default: dynamic)"}
                        },
                        "required": ["device_id"]
                    }
                },
                {
                    "name": "remove_device",
                    "description": "Remove a device by ID",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "device_id": {"type": "string", "description": "Device ID to remove"}
                        },
                        "required": ["device_id"]
                    }
                },
                {
                    "name": "list_folders",
                    "description": "List all configured folders",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "get_folder_status",
                    "description": "Get sync status of a specific folder",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "folder_id": {"type": "string", "description": "Folder identifier"}
                        },
                        "required": ["folder_id"]
                    }
                },
                {
                    "name": "add_folder",
                    "description": "Add a new sync folder",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "description": "Unique folder identifier"},
                            "path": {"type": "string", "description": "Absolute local path"},
                            "label": {"type": "string", "description": "Human-readable label (optional)"}
                        },
                        "required": ["id", "path"]
                    }
                },
                {
                    "name": "remove_folder",
                    "description": "Remove a folder by ID",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "folder_id": {"type": "string", "description": "Folder identifier to remove"}
                        },
                        "required": ["folder_id"]
                    }
                },
                {
                    "name": "get_system_status",
                    "description": "Get syncthing system status (uptime, version, myID)",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "get_connections",
                    "description": "Get current connection status to all devices",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "trigger_scan",
                    "description": "Trigger a folder rescan",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "folder_id": {"type": "string", "description": "Folder to scan (optional, scans all if omitted)"}
                        }
                    }
                }
            ]
        }))
    }

    async fn handle_tools_call(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params"))?;
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError::invalid_params("Missing tool name"))?;
        let args = params.get("arguments").cloned().unwrap_or(Value::Null);

        let response = match name {
            "list_devices" => self.rest_get("/rest/config/devices").await,
            "get_device" => {
                let id = arg_str(&args, "device_id")?;
                self.rest_get(&format!("/rest/device/{}", id)).await
            }
            "add_device" => {
                let body = serde_json::json!({
                    "id": arg_str(&args, "device_id")?,
                    "name": arg_opt_str(&args, "name"),
                    "addresses": arg_opt_array_str(&args, "addresses").unwrap_or_default(),
                });
                self.rest_post("/rest/devices", body).await
            }
            "remove_device" => {
                let id = arg_str(&args, "device_id")?;
                self.rest_delete(&format!("/rest/device/{}", id)).await
            }
            "list_folders" => self.rest_get("/rest/config/folders").await,
            "get_folder_status" => {
                let id = arg_str(&args, "folder_id")?;
                self.rest_get(&format!("/rest/folder/{}/status", id)).await
            }
            "add_folder" => {
                let body = serde_json::json!({
                    "id": arg_str(&args, "id")?,
                    "path": arg_str(&args, "path")?,
                    "label": arg_opt_str(&args, "label"),
                    "devices": [],
                });
                self.rest_post("/rest/folders", body).await
            }
            "remove_folder" => {
                let id = arg_str(&args, "folder_id")?;
                self.rest_delete(&format!("/rest/folder/{}", id)).await
            }
            "get_system_status" => self.rest_get("/rest/system/status").await,
            "get_connections" => self.rest_get("/rest/system/connections").await,
            "trigger_scan" => {
                if let Some(id) = arg_opt_str(&args, "folder_id") {
                    self.rest_post(&format!("/rest/scan/{}", id), Value::Null).await
                } else {
                    self.rest_post("/rest/scan", Value::Null).await
                }
            }
            _ => return Err(JsonRpcError::method_not_found()),
        };

        match response {
            Ok(text) => Ok(tool_result(text)),
            Err(e) => Ok(tool_error(e)),
        }
    }

    // ─── Resources ────────────────────────────────────────────────────────

    async fn handle_resources_list(&self) -> Result<Value, JsonRpcError> {
        Ok(serde_json::json!({
            "resources": [
                {
                    "uri": "syncthing://status",
                    "name": "System Status",
                    "mimeType": "application/json",
                    "description": "Current syncthing system status"
                },
                {
                    "uri": "syncthing://devices",
                    "name": "Devices",
                    "mimeType": "application/json",
                    "description": "Configured device list"
                },
                {
                    "uri": "syncthing://folders",
                    "name": "Folders",
                    "mimeType": "application/json",
                    "description": "Configured folder list"
                }
            ]
        }))
    }

    async fn handle_resources_read(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing params"))?;
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError::invalid_params("Missing resource uri"))?;

        let (endpoint, mime) = match uri {
            "syncthing://status" => ("/rest/system/status", "application/json"),
            "syncthing://devices" => ("/rest/config/devices", "application/json"),
            "syncthing://folders" => ("/rest/config/folders", "application/json"),
            _ => return Err(JsonRpcError::invalid_params("Unknown resource uri")),
        };

        match self.rest_get(endpoint).await {
            Ok(text) => Ok(serde_json::json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": mime,
                    "text": text
                }]
            })),
            Err(e) => Err(JsonRpcError::internal_error(e)),
        }
    }

    // ─── REST Helpers ─────────────────────────────────────────────────────

    async fn rest_get(&self, path: &str) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())
    }

    async fn rest_post(&self, path: &str, body: Value) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.client.post(&url);
        let req = if body.is_null() {
            req
        } else {
            req.json(&body)
        };
        req.send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())
    }

    async fn rest_delete(&self, path: &str) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        self.client
            .delete(&url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())
    }
}

// ─── Argument Helpers ─────────────────────────────────────────────────────

fn arg_str(args: &Value, key: &str) -> Result<String, JsonRpcError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| JsonRpcError::invalid_params(format!("Missing required argument: {}", key)))
}

fn arg_opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn arg_opt_array_str(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key)?.as_array().map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    })
}

// ─── MCP Result Builders ──────────────────────────────────────────────────

fn tool_result(text: String) -> Value {
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "isError": false
    })
}

fn tool_error(msg: String) -> Value {
    serde_json::json!({
        "content": [{"type": "text", "text": msg}],
        "isError": true
    })
}

// ─── Main ─────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .init();

    let base_url = env::var("SYNCTHING_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8385".into());
    tracing::info!("syncthing-mcp-bridge starting, target: {}", base_url);

    let server = McpServer::new(base_url);

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = tokio::io::BufReader::new(stdin);
    let mut stdout = stdout;
    let mut line = String::new();

    loop {
        line.clear();
        let n = tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await?;
        if n == 0 {
            tracing::info!("stdin closed, exiting");
            break;
        }

        let line = line.trim();
        // Strip UTF-8 BOM if present (common on Windows)
        let line = line.strip_prefix('\u{FEFF}').unwrap_or(line);
        if line.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to parse JSON-RPC: {}", e);
                let resp = JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: None,
                    result: None,
                    error: Some(JsonRpcError::parse_error(e.to_string())),
                };
                send_response(&mut stdout, &resp).await?;
                continue;
            }
        };

        tracing::debug!("← {:?}", req.method);

        if let Some(resp) = server.handle(req).await {
            send_response(&mut stdout, &resp).await?;
        }
    }

    Ok(())
}

async fn send_response(
    stdout: &mut tokio::io::Stdout,
    resp: &JsonRpcResponse,
) -> anyhow::Result<()> {
    let json = serde_json::to_string(resp)?;
    tokio::io::AsyncWriteExt::write_all(stdout, json.as_bytes()).await?;
    tokio::io::AsyncWriteExt::write_all(stdout, b"\n").await?;
    tokio::io::AsyncWriteExt::flush(stdout).await?;
    tracing::debug!("→ {}", json);
    Ok(())
}
