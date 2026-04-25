//! Global Discovery 客户端
//!
//! 与 Syncthing 官方发现服务器互通，支持 HTTPS + mTLS。
//! 协议规范见 `docs/design/NETWORK_DISCOVERY_DESIGN.md` §4。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde_json::json;
use tokio::sync::{broadcast, Notify};
use tracing::{debug, error, info, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

/// 默认官方发现服务器
pub const DEFAULT_DISCOVERY_SERVER: &str = "https://discovery.syncthing.net/v2/";

/// Announce 间隔（30 分钟）
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(1800);

/// 失败重试间隔（5 分钟）
pub const RETRY_INTERVAL: Duration = Duration::from_secs(300);



/// Global Discovery 客户端
///
/// 负责向发现服务器注册本机地址，并查询其他设备的地址。
/// mTLS 配置通过外部传入的 `reqwest::Client` 完成（daemon_runner 层配置）。
#[derive(Debug, Clone)]
pub struct GlobalDiscovery {
    server_url: String,
    device_id: DeviceId,
    client: Client,
    /// 外部触发 re-announce 的通知器
    notify: Arc<Notify>,
    /// 优雅退出信号发送端
    shutdown_tx: broadcast::Sender<()>,
}

impl GlobalDiscovery {
    /// 使用默认官方发现服务器创建
    pub fn new(device_id: DeviceId, client: Client) -> Self {
        Self::with_server(device_id, client, DEFAULT_DISCOVERY_SERVER.to_string())
    }

    /// 触发优雅退出
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    /// 获取 shutdown 发送端（用于外部 Drop guard）
    pub fn shutdown_sender(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    /// 获取通知器引用，用于外部触发 re-announce
    pub fn notifier(&self) -> Arc<Notify> {
        Arc::clone(&self.notify)
    }

    /// 使用自定义发现服务器创建
    pub fn with_server(device_id: DeviceId, client: Client, server_url: String) -> Self {
        // 确保 URL 以 / 结尾
        let server_url = if server_url.ends_with('/') {
            server_url
        } else {
            format!("{}/", server_url)
        };

        let (shutdown_tx, _) = broadcast::channel(1);
        Self {
            server_url,
            device_id,
            client,
            notify: Arc::new(Notify::new()),
            shutdown_tx,
        }
    }

    /// 从证书文件创建 GlobalDiscovery 客户端（自动配置 mTLS）
    pub async fn from_cert_files(
        device_id: DeviceId,
        cert_path: &Path,
        key_path: &Path,
        server_url: Option<String>,
    ) -> Result<Self> {
        let cert_pem = tokio::fs::read(cert_path)
            .await
            .map_err(|e| SyncthingError::config(format!("read cert.pem: {}", e)))?;
        let key_pem = tokio::fs::read(key_path)
            .await
            .map_err(|e| SyncthingError::config(format!("read key.pem: {}", e)))?;

        let mut identity_pem = cert_pem;
        identity_pem.extend_from_slice(&key_pem);

        let identity = reqwest::Identity::from_pem(&identity_pem)
            .map_err(|e| SyncthingError::Tls(format!("reqwest identity: {}", e)))?;

        let client = reqwest::Client::builder()
            .identity(identity)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| SyncthingError::network(format!("reqwest client: {}", e)))?;

        let server_url = server_url.unwrap_or_else(|| DEFAULT_DISCOVERY_SERVER.to_string());
        let (shutdown_tx, _) = broadcast::channel(1);
        Ok(Self {
            server_url,
            device_id,
            client,
            notify: Arc::new(Notify::new()),
            shutdown_tx,
        })
    }

    /// 触发一次立即 re-announce（外部调用，如 STUN/UPnP 发现新地址后）
    pub fn trigger_reannounce(&self) {
        self.notify.notify_one();
    }

    /// 向发现服务器注册地址
    ///
    /// # 参数
    /// - `addresses`：本机可达地址列表，如 `["tcp://192.168.1.10:22001", "tcp://10.0.0.5:22001"]`
    pub async fn announce(&self, addresses: &[String]) -> Result<()> {
        let url = format!("{}?device={}", self.server_url, self.device_id);
        let body = json!({ "addresses": addresses });

        debug!(
            "GlobalDiscovery announce to {} with {} addresses",
            url,
            addresses.len()
        );

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                SyncthingError::network(format!("Global discovery announce request failed: {}", e))
            })?;

        let status = resp.status();
        if status.is_success() {
            info!("Global discovery announce success ({})", status);
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            error!(
                "Global discovery announce failed: HTTP {} - {}",
                status, text
            );
            Err(SyncthingError::network(format!(
                "Global discovery announce failed: HTTP {} - {}",
                status, text
            )))
        }
    }

    /// 查询目标设备的地址
    ///
    /// # 参数
    /// - `target`：要查询的设备 ID
    ///
    /// # 返回
    /// 地址列表，如 `["tcp://203.0.113.5:22001", "relay://relay.syncthing.net:22067?id=..."]`
    pub async fn query(&self, target: DeviceId) -> Result<Vec<String>> {
        let url = format!("{}?device={}", self.server_url, target);

        debug!("GlobalDiscovery query for {} at {}", target, url);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| {
                SyncthingError::network(format!("Global discovery query request failed: {}", e))
            })?;

        let status = resp.status();
        if status.is_success() {
            let data: serde_json::Value = resp.json().await.map_err(|e| {
                SyncthingError::network(format!(
                    "Global discovery query JSON parse failed: {}",
                    e
                ))
            })?;

            let addresses: Vec<String> = data
                .get("addresses")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            info!(
                "Global discovery query for {} returned {} address(es)",
                target,
                addresses.len()
            );
            Ok(addresses)
        } else if status.as_u16() == 404 {
            // 设备未注册，返回空列表而非错误
            warn!("Global discovery query for {}: device not found (404)", target);
            Ok(Vec::new())
        } else {
            let text = resp.text().await.unwrap_or_default();
            error!(
                "Global discovery query failed: HTTP {} - {}",
                status, text
            );
            Err(SyncthingError::network(format!(
                "Global discovery query failed: HTTP {} - {}",
                status, text
            )))
        }
    }

    /// 运行后台 announce 循环
    ///
    /// 立即首次 announce（通常只包含本地地址），
    /// 之后每 `ANNOUNCE_INTERVAL` 向发现服务器注册一次。
    /// 失败时等待 `RETRY_INTERVAL` 后重试。
    /// 外部可通过 `trigger_reannounce()` 唤醒立即 announce（用于 STUN/UPnP 发现新地址后补发）。
    pub async fn run(&self, addresses: Arc<tokio::sync::Mutex<Vec<String>>>) {
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        loop {
            let addrs: Vec<String> = addresses.lock().await.clone();
            if addrs.is_empty() {
                warn!("Global discovery: no addresses to announce, skipping");
            } else {
                match self.announce(&addrs).await {
                    Ok(()) => {}
                    Err(e) => {
                        warn!(
                            "Global discovery announce failed: {}, retrying in {:?}",
                            e, RETRY_INTERVAL
                        );
                        tokio::time::sleep(RETRY_INTERVAL).await;
                        continue;
                    }
                }
            }

            // 等待 ANNOUNCE_INTERVAL、外部唤醒或优雅退出信号
            tokio::select! {
                _ = tokio::time::sleep(ANNOUNCE_INTERVAL) => {}
                _ = self.notify.notified() => {
                    info!("GlobalDiscovery re-announce triggered by address change");
                }
                _ = shutdown_rx.recv() => {
                    info!("GlobalDiscovery shutting down gracefully");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_discovery_url_trailing_slash() {
        let client = Client::builder().build().unwrap();
        let gd = GlobalDiscovery::with_server(
            DeviceId::default(),
            client.clone(),
            "https://example.com/v2".to_string(),
        );
        assert_eq!(gd.server_url, "https://example.com/v2/");

        let gd2 = GlobalDiscovery::with_server(
            DeviceId::default(),
            client,
            "https://example.com/v2/".to_string(),
        );
        assert_eq!(gd2.server_url, "https://example.com/v2/");
    }
}
