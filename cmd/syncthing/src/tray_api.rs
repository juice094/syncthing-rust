//! REST API 客户端 — 与 syncthing daemon 通信

use serde::Deserialize;

/// Daemon 状态摘要
#[derive(Debug, Clone, Default)]
pub struct DaemonStatus {
    pub online: bool,
    pub connected_devices: usize,
    pub total_devices: usize,
    pub folder_count: usize,
    pub syncing: bool,
    pub errors: Vec<String>,
}

/// 轻量级 REST 客户端
pub struct DaemonClient {
    base_url: String,
    status: DaemonStatus,
}

#[derive(Debug, Deserialize)]
struct Connections {
    total: ConnectionTotals,
}

#[derive(Debug, Deserialize)]
struct ConnectionTotals {
    #[serde(rename = "connectedDevices")]
    connected_devices: i32,
}

#[derive(Debug, Deserialize)]
struct FolderStatusItem {
    #[serde(default)]
    pull_errors: u64,
    #[serde(default)]
    need_bytes: u64,
    #[serde(default)]
    state: Option<String>,
}

impl DaemonClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            status: DaemonStatus::default(),
        }
    }

    /// Ping daemon 检查是否存活
    pub async fn ping(&self) -> bool {
        match reqwest::get(format!("{}/rest/health", self.base_url)).await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// 更新在线状态
    pub fn set_online(&mut self, online: bool) {
        self.status.online = online;
    }

    /// 刷新状态（设备连接数、文件夹数、同步状态、错误等）
    pub async fn refresh_status(&mut self) -> anyhow::Result<()> {
        if !self.status.online {
            return Ok(());
        }

        // 并行获取 connections 和 folders 列表
        let conn_fut = reqwest::get(format!("{}/rest/system/connections", self.base_url));
        let folders_fut = reqwest::get(format!("{}/rest/config/folders", self.base_url));
        let (conn_resp, folders_resp) = tokio::join!(conn_fut, folders_fut);

        // 连接统计
        match conn_resp {
            Ok(resp) => {
                if let Ok(data) = resp.json::<Connections>().await {
                    self.status.connected_devices = data.total.connected_devices.max(0) as usize;
                }
            }
            Err(e) => {
                tracing::debug!("connections request failed: {}", e);
            }
        }

        // 文件夹列表 + 逐文件夹状态检查
        if let Ok(resp) = folders_resp {
            if let Ok(arr) = resp.json::<Vec<serde_json::Value>>().await {
                self.status.folder_count = arr.len();
                self.status.total_devices = self.status.connected_devices; // best-effort

                // 检查每个文件夹的同步状态
                let mut syncing = false;
                let mut errors = Vec::new();
                for folder in &arr {
                    if let Some(folder_id) = folder.get("id").and_then(|v| v.as_str()) {
                        if let Ok(resp) = reqwest::get(format!(
                            "{}/rest/folder/{}/status",
                            self.base_url, folder_id
                        ))
                        .await
                        {
                            if let Ok(s) = resp.json::<FolderStatusItem>().await {
                                if s.pull_errors > 0 {
                                    errors.push(format!(
                                        "{}: {} pull errors",
                                        folder_id, s.pull_errors
                                    ));
                                }
                                if s.need_bytes > 0 || s.state.as_deref() == Some("syncing") {
                                    syncing = true;
                                }
                            }
                        }
                    }
                }
                self.status.syncing = syncing;
                self.status.errors = errors;
            }
        }

        Ok(())
    }

    pub fn status(&self) -> &DaemonStatus {
        &self.status
    }

    /// Determine tray icon type from current status.
    pub fn icon_type(&self) -> crate::tray::IconType {
        if !self.status.online {
            crate::tray::IconType::Error
        } else if self.status.syncing {
            crate::tray::IconType::Syncing
        } else if self.status.connected_devices > 0 {
            crate::tray::IconType::Idle
        } else {
            crate::tray::IconType::Default
        }
    }
}
