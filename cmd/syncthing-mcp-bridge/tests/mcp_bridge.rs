//! syncthing-mcp-bridge 集成测试
//!
//! 用真实二进制 + 进程内 stub HTTP 服务端验证 stdio JSON-RPC 到 REST 的完整链路：
//! 1. `tools/list` 的工具名册完整；
//! 2. `tools/call` 把请求落到正确的 REST 路径，并原样返回响应体；
//! 3. `SYNCTHING_API_KEY` 会作为 `X-API-Key` 头转发（非 loopback 部署所需）；
//! 4. 带特殊字符的取值会被百分号编码（不会因空格/`&` 破坏请求）。
//!
//! stub 服务端刻意不引入任何 HTTP 框架依赖：手写最小 HTTP/1.1 应答，
//! 只解析请求行与 header，并记录收到的请求以供断言。

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const STUB_BODY: &str = r#"{"status":"up"}"#;

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: String,
    path: String,
    api_key: Option<String>,
}

/// 启动 stub HTTP 服务端，返回 (base_url, 已捕获请求表)。
async fn spawn_stub_server() -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let captured: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_c = Arc::clone(&captured);

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let captured = Arc::clone(&captured_c);
            tokio::spawn(async move {
                // 读到 header 结束（\r\n\r\n）为止；本桥不发送请求体。
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    match socket.read(&mut chunk).await {
                        Ok(0) => return,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => return,
                    }
                }

                let text = String::from_utf8_lossy(&buf).to_string();
                let mut lines = text.lines();
                let request_line = lines.next().unwrap_or_default();
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let api_key = text.lines().find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    if key.eq_ignore_ascii_case("x-api-key") {
                        Some(value.trim().to_string())
                    } else {
                        None
                    }
                });

                captured
                    .lock()
                    .expect("captured lock")
                    .push(CapturedRequest {
                        method,
                        path,
                        api_key,
                    });

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    STUB_BODY.len(),
                    STUB_BODY
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (format!("http://{}", addr), captured)
}

struct Bridge {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
}

impl Bridge {
    async fn spawn(base_url: &str, api_key: Option<&str>) -> Self {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_syncthing-mcp-bridge"));
        cmd.env("SYNCTHING_API_URL", base_url);
        match api_key {
            Some(key) => {
                cmd.env("SYNCTHING_API_KEY", key);
            }
            None => {
                cmd.env_remove("SYNCTHING_API_KEY");
            }
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().expect("spawn bridge binary");
        let stdin = child.stdin.take().expect("bridge stdin");
        let stdout = tokio::io::BufReader::new(child.stdout.take().expect("bridge stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    /// 发送一条 JSON-RPC 请求并读取一行响应。
    async fn request(&mut self, request: &str) -> serde_json::Value {
        self.stdin
            .write_all(request.as_bytes())
            .await
            .expect("write request");
        self.stdin.write_all(b"\n").await.expect("write newline");
        self.stdin.flush().await.expect("flush request");

        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .await
            .expect("read response line");
        serde_json::from_str(line.trim()).expect("parse JSON-RPC response")
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn tool_names(list_response: &serde_json::Value) -> Vec<String> {
    list_response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().expect("tool name").to_string())
        .collect()
}

/// `tools/list` 必须暴露完整工具名册（含本轮新增的运维/观测类工具）。
#[tokio::test]
async fn tools_list_exposes_full_toolset() {
    let (_base_url, _captured) = spawn_stub_server().await;
    let mut bridge = Bridge::spawn(&_base_url, None).await;

    let response = bridge
        .request(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
        .await;

    let names = tool_names(&response);
    for expected in [
        "list_devices",
        "get_device",
        "add_device",
        "remove_device",
        "list_folders",
        "get_folder_status",
        "add_folder",
        "remove_folder",
        "get_system_status",
        "get_connections",
        "trigger_scan",
        "get_health",
        "get_metrics",
        "get_db_status",
        "browse_folder",
        "get_file",
        "pause_folder",
        "resume_folder",
        "poll_events",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "tool `{}` missing from tools/list: {:?}",
            expected,
            names
        );
    }
    assert_eq!(names.len(), 19, "unexpected tool count: {:?}", names);

    // 资源名册保持 3 条不变
    let resources = bridge
        .request(r#"{"jsonrpc":"2.0","id":2,"method":"resources/list","params":{}}"#)
        .await;
    assert_eq!(
        resources["result"]["resources"].as_array().unwrap().len(),
        3
    );
}

/// `tools/call get_health` 落到 `/rest/health`，并把响应体原样放进 content[0].text；
/// 同时验证 `SYNCTHING_API_KEY` 以 `X-API-Key` 头转发。
#[tokio::test]
async fn tools_call_proxies_rest_and_forwards_api_key() {
    let (base_url, captured) = spawn_stub_server().await;
    let mut bridge = Bridge::spawn(&base_url, Some("test-api-key")).await;

    let response = bridge
        .request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_health","arguments":{}}}"#,
        )
        .await;

    assert_eq!(response["result"]["isError"], serde_json::json!(false));
    assert_eq!(
        response["result"]["content"][0]["text"].as_str().unwrap(),
        STUB_BODY
    );

    let requests = captured.lock().expect("captured lock").clone();
    assert_eq!(requests.len(), 1, "expected exactly one REST call");
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/rest/health");
    assert_eq!(
        requests[0].api_key.as_deref(),
        Some("test-api-key"),
        "X-API-Key header must be forwarded"
    );

    drop(bridge);
}

/// 查询参数与路径段中的特殊字符必须做百分号编码，否则请求会被空格/`&` 截断。
#[tokio::test]
async fn tools_call_encodes_query_and_path_values() {
    let (base_url, captured) = spawn_stub_server().await;
    let mut bridge = Bridge::spawn(&base_url, None).await;

    bridge
        .request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browse_folder","arguments":{"folder_id":"my folder","prefix":"a/b c","levels":2}}}"#,
        )
        .await;
    bridge
        .request(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"pause_folder","arguments":{"folder_id":"my folder"}}}"#,
        )
        .await;

    let requests = captured.lock().expect("captured lock").clone();
    assert_eq!(requests.len(), 2, "expected two REST calls");

    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/rest/db/browse?folder=my%20folder&prefix=a%2Fb%20c&levels=2"
    );
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/rest/pause/my%20folder");

    drop(bridge);
}

/// 未配置 `SYNCTHING_API_KEY` 时不得发送鉴权头（保持 loopback 免鉴权行为不变）。
#[tokio::test]
async fn api_key_absent_when_not_configured() {
    let (base_url, captured) = spawn_stub_server().await;
    let mut bridge = Bridge::spawn(&base_url, None).await;

    bridge
        .request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_metrics","arguments":{}}}"#,
        )
        .await;

    let requests = captured.lock().expect("captured lock").clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/metrics");
    assert_eq!(requests[0].api_key, None);

    drop(bridge);
}
