//! syncthing-mcp-bridge
//!
//! MCP (Model Context Protocol) Bridge for syncthing-rust.
//! Translates MCP stdio JSON-RPC messages into REST API calls.
//!
//! Architecture: Kimi/Claude ↔ MCP stdio ↔ this bridge ↔ HTTP REST ↔ syncthing-rust

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;

mod tools;

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
    /// 可选的 REST API 管理员 Key（来源：环境变量 `SYNCTHING_API_KEY`）。
    ///
    /// REST API 对 loopback 来源免鉴权，因此默认（未设置）时行为不变；当 daemon 绑定到
    /// 非 loopback 地址（`config.gui.address`）时必须携带 `X-API-Key` 才能访问。
    api_key: Option<String>,
}

impl McpServer {
    fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            api_key,
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
        let is_notification = req.id.is_none() || req.id.as_ref().is_some_and(|v| v.is_null());

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
        Ok(tools::catalog())
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
                    self.rest_post(&format!("/rest/scan/{}", id), Value::Null)
                        .await
                } else {
                    self.rest_post("/rest/scan", Value::Null).await
                }
            }
            "get_health" => self.rest_get("/rest/health").await,
            "get_metrics" => self.rest_get("/metrics").await,
            "get_db_status" => self.rest_get("/rest/db/status").await,
            "browse_folder" => {
                let mut params: Vec<(String, String)> =
                    vec![("folder".to_string(), arg_str(&args, "folder_id")?)];
                if let Some(prefix) = arg_opt_str(&args, "prefix") {
                    params.push(("prefix".to_string(), prefix));
                }
                if let Some(levels) = arg_opt_u64(&args, "levels") {
                    params.push(("levels".to_string(), levels.to_string()));
                }
                let path = format!("/rest/db/browse?{}", encode_query(&params));
                self.rest_get(&path).await
            }
            "get_file" => {
                let params = vec![
                    ("folder".to_string(), arg_str(&args, "folder_id")?),
                    ("file".to_string(), arg_str(&args, "file")?),
                ];
                let path = format!("/rest/db/file?{}", encode_query(&params));
                self.rest_get(&path).await
            }
            "pause_folder" => {
                let id = arg_str(&args, "folder_id")?;
                self.rest_post(
                    &format!("/rest/pause/{}", encode_component(&id)),
                    Value::Null,
                )
                .await
            }
            "resume_folder" => {
                let id = arg_str(&args, "folder_id")?;
                self.rest_post(
                    &format!("/rest/resume/{}", encode_component(&id)),
                    Value::Null,
                )
                .await
            }
            "poll_events" => {
                let mut params: Vec<(String, String)> = Vec::new();
                if let Some(since) = arg_opt_u64(&args, "since") {
                    params.push(("since".to_string(), since.to_string()));
                }
                if let Some(limit) = arg_opt_u64(&args, "limit") {
                    params.push(("limit".to_string(), limit.to_string()));
                }
                if let Some(events) = arg_opt_str(&args, "events") {
                    params.push(("events".to_string(), events));
                }
                let path = if params.is_empty() {
                    "/rest/events/poll".to_string()
                } else {
                    format!("/rest/events/poll?{}", encode_query(&params))
                };
                self.rest_get(&path).await
            }
            _ => return Err(JsonRpcError::method_not_found()),
        };

        match response {
            Ok(text) => Ok(tool_result(&text)),
            Err(e) => Ok(tool_error(&e)),
        }
    }

    // ─── Resources ────────────────────────────────────────────────────────

    async fn handle_resources_list(&self) -> Result<Value, JsonRpcError> {
        Ok(tools::resources())
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

    /// 需要鉴权时附加 `X-API-Key`（loopback 访问由服务端放行，无需该 header）。
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(key) => req.header("X-API-Key", key),
            None => req,
        }
    }

    async fn rest_get(&self, path: &str) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        self.auth(self.client.get(&url))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())
    }

    async fn rest_post(&self, path: &str, body: Value) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let req = self.auth(self.client.post(&url));
        let req = if body.is_null() { req } else { req.json(&body) };
        req.send()
            .await
            .map_err(|e| e.to_string())?
            .text()
            .await
            .map_err(|e| e.to_string())
    }

    async fn rest_delete(&self, path: &str) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        self.auth(self.client.delete(&url))
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
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn arg_opt_array_str(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key)?.as_array().map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    })
}

fn arg_opt_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key)?.as_u64()
}

/// RFC 3986 unreserved 集合之外的字节一律百分号编码，用于把 folder/file 等
/// 取值安全嵌入 URL 路径段或查询串（避免空格、`&`、`#`、`?` 破坏请求）。
fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 构造 `k=v&k2=v2` 形式的查询串（键与值均做百分号编码）。
fn encode_query(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", encode_component(k), encode_component(v)))
        .collect::<Vec<_>>()
        .join("&")
}

// ─── MCP Result Builders ──────────────────────────────────────────────────

fn tool_result(text: &str) -> Value {
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "isError": false
    })
}

fn tool_error(msg: &str) -> Value {
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
    // loopback 目标无需鉴权；绑定到非 loopback 地址时用该 Key 通过 REST 鉴权。
    let api_key = env::var("SYNCTHING_API_KEY").ok().filter(|k| !k.is_empty());
    tracing::info!(
        "syncthing-mcp-bridge starting, target: {} (api key: {})",
        base_url,
        if api_key.is_some() { "set" } else { "none" }
    );

    let server = McpServer::new(base_url, api_key);

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
