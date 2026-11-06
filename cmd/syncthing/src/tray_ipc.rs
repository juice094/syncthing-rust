//! IPC between the tray process and TUI processes.
//!
//! A real named-pipe implementation is provided on Windows with the `tray`
//! feature; other platforms get no-op stubs so the code compiles everywhere.

use serde::{Deserialize, Serialize};

#[cfg(all(windows, feature = "tray"))]
use std::path::{Path, PathBuf};

#[cfg(all(windows, feature = "tray"))]
const TRAY_PIPE_FILE_NAME: &str = "tray.pipe";

/// Commands sent from a TUI to the tray.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "payload")]
pub enum TrayIpcRequest {
    /// Heartbeat / liveness probe.
    Ping,
    /// Ask the tray to start the daemon.
    StartDaemon,
    /// Ask the tray to stop the daemon.
    StopDaemon,
    /// Ask the tray to open or focus the TUI window.
    OpenTui,
}

/// Path of the file persisting the tray IPC pipe name.
#[cfg(all(windows, feature = "tray"))]
pub fn tray_pipe_path(config_dir: &Path) -> PathBuf {
    config_dir.join(TRAY_PIPE_FILE_NAME)
}

/// Writes the current tray IPC pipe name for newly started processes to discover.
#[cfg(all(windows, feature = "tray"))]
pub fn write_tray_pipe(config_dir: &Path, pipe_name: &str) -> std::io::Result<()> {
    let path = tray_pipe_path(config_dir);
    std::fs::create_dir_all(config_dir)?;
    std::fs::write(&path, pipe_name)
}

/// Reads the persisted tray IPC pipe name, if present and non-empty.
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

/// Removes the persisted tray IPC pipe name file.
#[cfg(all(windows, feature = "tray"))]
pub fn clear_tray_pipe(config_dir: &Path) {
    let _ = std::fs::remove_file(tray_pipe_path(config_dir));
}

/// Response sent from the tray back to a TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayIpcResponse {
    pub ok: bool,
    pub error: Option<String>,
}

#[allow(dead_code)]
impl TrayIpcResponse {
    /// Builds a success response.
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
        }
    }

    /// Builds an error response carrying `e`'s text.
    pub fn err(e: impl ToString) -> Self {
        Self {
            ok: false,
            error: Some(e.to_string()),
        }
    }
}

/// Errors from tray IPC pipe operations.
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

    /// Generates a unique pipe name.
    ///
    /// Embeds the process PID and a random nonce to reduce the chance of
    /// cross-session / cross-user collisions.
    pub fn generate_pipe_name() -> String {
        let pid = std::process::id();
        let nonce = rand::random::<u32>();
        format!(r"\\.\pipe\syncthing-rust-tray-{pid}-{nonce}")
    }

    /// Named-pipe server forwarding decoded commands to the tray main loop.
    pub struct TrayIpcServer {
        pipe_name: String,
        cmd_tx: mpsc::Sender<TrayIpcRequest>,
    }

    impl TrayIpcServer {
        /// Creates a server bound to `pipe_name`.
        pub fn new(pipe_name: impl Into<String>, cmd_tx: mpsc::Sender<TrayIpcRequest>) -> Self {
            Self {
                pipe_name: pipe_name.into(),
                cmd_tx,
            }
        }

        /// Runs the accept/read loop until the pipe breaks.
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

                // Current client disconnected; recreate the server instance for the next one.
                server = ServerOptions::new().create(&self.pipe_name)?;
            }
        }
    }

    /// Named-pipe client used by TUI processes to command the tray.
    pub struct TrayIpcClient {
        writer: WriteHalf<NamedPipeClient>,
        reader: BufReader<ReadHalf<NamedPipeClient>>,
    }

    impl TrayIpcClient {
        /// Connects to the tray's pipe.
        pub async fn connect(pipe_name: impl AsRef<str>) -> Result<Self, TrayIpcError> {
            let client = ClientOptions::new().open(pipe_name.as_ref())?;
            let (reader, writer) = split(client);
            Ok(Self {
                writer,
                reader: BufReader::new(reader),
            })
        }

        /// Sends one request and waits for the response (5s timeout).
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
#[allow(dead_code)]
mod imp {
    use super::*;
    use tokio::sync::mpsc;

    /// Stub: returns an empty pipe name on platforms without tray IPC.
    pub fn generate_pipe_name() -> String {
        String::new()
    }

    /// Stub server that never accepts a connection.
    pub struct TrayIpcServer {
        _cmd_tx: mpsc::Sender<TrayIpcRequest>,
    }

    impl TrayIpcServer {
        /// Creates the stub server (arguments ignored).
        pub fn new(_pipe_name: impl Into<String>, _cmd_tx: mpsc::Sender<TrayIpcRequest>) -> Self {
            Self { _cmd_tx }
        }

        /// Parks forever; tray IPC is unsupported on this platform.
        pub async fn run(self) -> Result<(), TrayIpcError> {
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    /// Stub client that always fails with `NotSupported`.
    pub struct TrayIpcClient;

    impl TrayIpcClient {
        /// Fails with `TrayIpcError::NotSupported`.
        pub async fn connect(_pipe_name: impl AsRef<str>) -> Result<Self, TrayIpcError> {
            Err(TrayIpcError::NotSupported)
        }

        /// Fails with `TrayIpcError::NotSupported`.
        pub async fn send(
            &mut self,
            _req: TrayIpcRequest,
        ) -> Result<TrayIpcResponse, TrayIpcError> {
            Err(TrayIpcError::NotSupported)
        }
    }
}

pub use imp::*;
