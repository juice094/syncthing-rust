//! 托盘进程与 TUI 之间的 IPC。
//!
//! 仅在 Windows + tray feature 下提供真正的命名管道实现；
//! 其他平台为 no-op 桩，保证代码跨平台可编译。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const TRAY_PIPE_FILE_NAME: &str = "tray.pipe";

/// TUI → 托盘的命令。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "payload")]
pub enum TrayIpcRequest {
    /// 心跳/探测
    Ping,
    /// 通知托盘启动 daemon
    StartDaemon,
    /// 通知托盘停止 daemon
    StopDaemon,
    /// 通知托盘打开或聚焦 TUI 窗口
    OpenTui,
}

/// 托盘 IPC 管道名持久化文件路径。
#[cfg(all(windows, feature = "tray"))]
pub fn tray_pipe_path(config_dir: &Path) -> PathBuf {
    config_dir.join(TRAY_PIPE_FILE_NAME)
}

/// 将当前托盘 IPC 管道名写入持久化文件，供外部新进程发现。
#[cfg(all(windows, feature = "tray"))]
pub fn write_tray_pipe(config_dir: &Path, pipe_name: &str) -> std::io::Result<()> {
    let path = tray_pipe_path(config_dir);
    std::fs::create_dir_all(config_dir)?;
    std::fs::write(&path, pipe_name)
}

/// 读取持久化的托盘 IPC 管道名（若存在且非空）。
#[cfg(all(windows, feature = "tray"))]
pub fn read_tray_pipe(config_dir: &Path) -> Option<String> {
    let path = tray_pipe_path(config_dir);
    let content = std::fs::read_to_string(&path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 清理托盘 IPC 管道名持久化文件。
#[cfg(all(windows, feature = "tray"))]
pub fn clear_tray_pipe(config_dir: &Path) {
    let _ = std::fs::remove_file(tray_pipe_path(config_dir));
}

/// 托盘 → TUI 的响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayIpcResponse {
    pub ok: bool,
    pub error: Option<String>,
}

#[allow(dead_code)]
impl TrayIpcResponse {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
        }
    }

    pub fn err(e: impl ToString) -> Self {
        Self {
            ok: false,
            error: Some(e.to_string()),
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum TrayIpcError {
    NotSupported,
    Disconnected,
    Io(std::io::Error),
    Serialization(serde_json::Error),
    Timeout,
}

impl std::fmt::Display for TrayIpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrayIpcError::NotSupported => write!(f, "not supported on this platform"),
            TrayIpcError::Disconnected => write!(f, "pipe disconnected"),
            TrayIpcError::Io(e) => write!(f, "io error: {}", e),
            TrayIpcError::Serialization(e) => write!(f, "serialization error: {}", e),
            TrayIpcError::Timeout => write!(f, "timeout"),
        }
    }
}

impl std::error::Error for TrayIpcError {}

impl From<std::io::Error> for TrayIpcError {
    fn from(e: std::io::Error) -> Self {
        TrayIpcError::Io(e)
    }
}

impl From<serde_json::Error> for TrayIpcError {
    fn from(e: serde_json::Error) -> Self {
        TrayIpcError::Serialization(e)
    }
}

#[cfg(all(windows, feature = "tray"))]
mod imp {
    use super::*;
    use std::time::Duration;
    use tokio::io::{split, AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, ServerOptions};
    use tokio::sync::mpsc;

    const PIPE_TIMEOUT: Duration = Duration::from_secs(5);

    /// 生成唯一管道名。
    ///
    /// 包含进程 PID 与随机 nonce，降低跨会话/跨用户冲突概率。
    pub fn generate_pipe_name() -> String {
        let pid = std::process::id();
        let nonce = rand::random::<u32>();
        format!(r"\\.\pipe\syncthing-rust-tray-{pid}-{nonce}")
    }

    pub struct TrayIpcServer {
        pipe_name: String,
        cmd_tx: mpsc::Sender<TrayIpcRequest>,
    }

    impl TrayIpcServer {
        pub fn new(pipe_name: impl Into<String>, cmd_tx: mpsc::Sender<TrayIpcRequest>) -> Self {
            Self {
                pipe_name: pipe_name.into(),
                cmd_tx,
            }
        }

        pub async fn run(self) -> Result<(), TrayIpcError> {
            let mut server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&self.pipe_name)?;

            loop {
                server.connect().await?;
                let connected = server;
                let (reader, mut writer) = split(connected);
                let mut buf_reader = BufReader::new(reader);
                let mut line = String::new();

                while buf_reader.read_line(&mut line).await? > 0 {
                    let req: TrayIpcRequest = serde_json::from_str(&line)?;
                    let is_ping = matches!(req, TrayIpcRequest::Ping);
                    let resp = if is_ping {
                        TrayIpcResponse::ok()
                    } else if self.cmd_tx.send(req).await.is_err() {
                        TrayIpcResponse::err("daemon controller unavailable")
                    } else {
                        TrayIpcResponse::ok()
                    };

                    let mut resp_line = serde_json::to_string(&resp)?;
                    resp_line.push('\n');
                    writer.write_all(resp_line.as_bytes()).await?;
                    writer.flush().await?;
                    line.clear();
                }

                // 当前客户端断开，重新创建服务端实例
                server = ServerOptions::new().create(&self.pipe_name)?;
            }
        }
    }

    pub struct TrayIpcClient {
        writer: WriteHalf<NamedPipeClient>,
        reader: BufReader<ReadHalf<NamedPipeClient>>,
    }

    impl TrayIpcClient {
        pub async fn connect(pipe_name: impl AsRef<str>) -> Result<Self, TrayIpcError> {
            let client = ClientOptions::new().open(pipe_name.as_ref())?;
            let (reader, writer) = split(client);
            Ok(Self {
                writer,
                reader: BufReader::new(reader),
            })
        }

        pub async fn send(&mut self, req: TrayIpcRequest) -> Result<TrayIpcResponse, TrayIpcError> {
            let mut line = serde_json::to_string(&req)?;
            line.push('\n');
            self.writer.write_all(line.as_bytes()).await?;
            self.writer.flush().await?;

            let mut line = String::new();
            tokio::time::timeout(PIPE_TIMEOUT, self.reader.read_line(&mut line))
                .await
                .map_err(|_| TrayIpcError::Timeout)??;

            let resp: TrayIpcResponse = serde_json::from_str(&line)?;
            Ok(resp)
        }
    }
}

#[cfg(not(all(windows, feature = "tray")))]
mod imp {
    use super::*;
    use tokio::sync::mpsc;

    pub fn generate_pipe_name() -> String {
        String::new()
    }

    pub struct TrayIpcServer {
        _cmd_tx: mpsc::Sender<TrayIpcRequest>,
    }

    impl TrayIpcServer {
        pub fn new(_pipe_name: impl Into<String>, _cmd_tx: mpsc::Sender<TrayIpcRequest>) -> Self {
            Self { _cmd_tx }
        }

        pub async fn run(self) -> Result<(), TrayIpcError> {
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    pub struct TrayIpcClient;

    impl TrayIpcClient {
        pub async fn connect(_pipe_name: impl AsRef<str>) -> Result<Self, TrayIpcError> {
            Err(TrayIpcError::NotSupported)
        }

        pub async fn send(
            &mut self,
            _req: TrayIpcRequest,
        ) -> Result<TrayIpcResponse, TrayIpcError> {
            Err(TrayIpcError::NotSupported)
        }
    }
}

pub use imp::*;
