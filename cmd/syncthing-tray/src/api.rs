//! REST API 客户端 — 与 syncthing daemon 通信

use serde::Deserialize;

/// Daemon 状态摘要
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
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
#[allow(dead_code)]
struct SystemStatus {
    #[serde(rename = "myID")]
    my_id: String,
    uptime: u64,
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
    #[allow(dead_code)]
    pub fn set_online(&mut self, online: bool) {
        self.status.online = online;
    }

    /// 刷新状态（设备连接数、文件夹数等）
    pub async fn refresh_status(&mut self) -> anyhow::Result<()> {
        if !self.status.online {
            return Ok(());
        }

        // 获取连接统计
        match reqwest::get(format!("{}/rest/system/connections", self.base_url)).await {
            Ok(resp) => {
                if let Ok(data) = resp.json::<Connections>().await {
                    self.status.connected_devices = data.total.connected_devices.max(0) as usize;
                }
            }
            Err(e) => {
                tracing::debug!("connections request failed: {}", e);
            }
        }

        // 获取文件夹列表以计算总数
        match reqwest::get(format!("{}/rest/config/folders", self.base_url)).await {
            Ok(resp) => {
                if let Ok(arr) = resp.json::<Vec<serde_json::Value>>().await {
                    self.status.folder_count = arr.len();
                }
            }
            Err(e) => {
                tracing::debug!("folders request failed: {}", e);
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
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
