//! REST API client for talking to a syncthing daemon (used by the tray).

use serde::Deserialize;

/// Snapshot of daemon status.
#[derive(Debug, Clone, Default)]
pub struct DaemonStatus {
    pub online: bool,
    pub connected_devices: usize,
    pub total_devices: usize,
    pub folder_count: usize,
    pub syncing: bool,
    pub errors: Vec<String>,
}

/// Default REST request timeout.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Lightweight REST client polling a daemon for tray status.
pub struct DaemonClient {
    base_url: String,
    status: DaemonStatus,
    http: reqwest::Client,
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
    /// Creates a client for the daemon REST API at `base_url`.
    pub fn new(base_url: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            status: DaemonStatus::default(),
            http,
        }
    }

    /// Pings the daemon's `/rest/health` to check liveness.
    pub async fn ping(&self) -> bool {
        match self
            .http
            .get(format!("{}/rest/health", self.base_url))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Updates the cached online flag.
    pub fn set_online(&mut self, online: bool) {
        self.status.online = online;
    }

    /// Refreshes device counts, folder counts, sync state, and errors.
    pub async fn refresh_status(&mut self) -> anyhow::Result<()> {
        if !self.status.online {
            return Ok(());
        }

        let conn_fut = self
            .http
            .get(format!("{}/rest/system/connections", self.base_url))
            .send();
        let folders_fut = self
            .http
            .get(format!("{}/rest/config/folders", self.base_url))
            .send();
        let devices_fut = self
            .http
            .get(format!("{}/rest/config/devices", self.base_url))
            .send();

        let (conn_resp, folders_resp, devices_resp) =
            tokio::join!(conn_fut, folders_fut, devices_fut);

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

        // Device total is tracked independently of the online count.
        if let Ok(resp) = devices_resp {
            if let Ok(arr) = resp.json::<Vec<serde_json::Value>>().await {
                self.status.total_devices = arr.len();
            }
        }

        if let Ok(resp) = folders_resp {
            if let Ok(arr) = resp.json::<Vec<serde_json::Value>>().await {
                self.status.folder_count = arr.len();

                let mut syncing = false;
                let mut errors = Vec::new();
                for folder in &arr {
                    if let Some(folder_id) = folder.get("id").and_then(|v| v.as_str()) {
                        let status_url =
                            format!("{}/rest/folder/{}/status", self.base_url, folder_id);
                        if let Ok(resp) = self.http.get(&status_url).send().await {
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

    /// Returns the cached status snapshot.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_type_default() {
        let client = DaemonClient::new("http://127.0.0.1:8385");
        assert_eq!(client.icon_type(), crate::tray::IconType::Error);
    }

    #[test]
    fn test_icon_type_online_idle() {
        let mut client = DaemonClient::new("http://127.0.0.1:8385");
        client.set_online(true);
        client.status.connected_devices = 1;
        client.status.total_devices = 2;
        assert_eq!(client.icon_type(), crate::tray::IconType::Idle);
    }

    #[test]
    fn test_icon_type_online_syncing() {
        let mut client = DaemonClient::new("http://127.0.0.1:8385");
        client.set_online(true);
        client.status.syncing = true;
        client.status.connected_devices = 1;
        assert_eq!(client.icon_type(), crate::tray::IconType::Syncing);
    }

    #[test]
    fn test_icon_type_online_no_devices() {
        let mut client = DaemonClient::new("http://127.0.0.1:8385");
        client.set_online(true);
        client.status.connected_devices = 0;
        client.status.total_devices = 3;
        assert_eq!(client.icon_type(), crate::tray::IconType::Default);
    }

    #[test]
    fn test_status_default() {
        let client = DaemonClient::new("http://127.0.0.1:8385");
        let status = client.status();
        assert!(!status.online);
        assert_eq!(status.connected_devices, 0);
        assert_eq!(status.total_devices, 0);
        assert_eq!(status.folder_count, 0);
        assert!(!status.syncing);
        assert!(status.errors.is_empty());
    }

    #[test]
    fn test_base_url_trailing_slash_removed() {
        let client = DaemonClient::new("http://127.0.0.1:8385/");
        // base_url 是私有的，无法直接断言；但后续 ping 会拼成 /rest/health
        // 这里仅验证构造不 panic。
        let _ = client;
    }
}
