//! Global Discovery client.
//!
//! Interoperates with the official Syncthing discovery servers over HTTPS + mTLS.
//! Protocol spec: `docs/design/NETWORK_DISCOVERY_DESIGN.md` §4.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde_json::json;
use tokio::sync::{broadcast, Notify};
use tracing::{debug, error, info, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

/// Default official discovery server.
pub const DEFAULT_DISCOVERY_SERVER: &str = "https://discovery.syncthing.net/v2/";

/// Announce interval (30 minutes).
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(1800);

/// Retry interval after failures (5 minutes).
pub const RETRY_INTERVAL: Duration = Duration::from_secs(300);

/// Global Discovery client.
///
/// Registers this device's addresses with the discovery server and queries
/// other devices' addresses. mTLS is configured through the externally
/// provided `reqwest::Client` (wired in the daemon_runner layer).
#[derive(Debug, Clone)]
pub struct GlobalDiscovery {
    server_url: String,
    device_id: DeviceId,
    client: Client,
    /// Notifier for externally triggered re-announces.
    notify: Arc<Notify>,
    /// Graceful shutdown signal sender.
    shutdown_tx: broadcast::Sender<()>,
}

impl GlobalDiscovery {
    /// Creates a client targeting the default official discovery server.
    pub fn new(device_id: DeviceId, client: Client) -> Self {
        Self::with_server(device_id, client, DEFAULT_DISCOVERY_SERVER.to_string())
    }

    /// Triggers graceful shutdown of the announce loop.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    /// Returns the shutdown sender (for external drop guards).
    pub fn shutdown_sender(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    /// Returns the notifier reference used to trigger re-announces.
    pub fn notifier(&self) -> Arc<Notify> {
        Arc::clone(&self.notify)
    }

    /// Creates a client targeting a custom discovery server.
    pub fn with_server(device_id: DeviceId, client: Client, server_url: String) -> Self {
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

    /// Creates a GlobalDiscovery client from certificate files (configures mTLS).
    ///
    /// # Errors
    /// Returns an error if the cert/key files cannot be read, the identity is
    /// invalid, or the HTTP client cannot be built.
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

    /// Triggers an immediate re-announce (e.g. after STUN/UPnP learns new addresses).
    pub fn trigger_reannounce(&self) {
        self.notify.notify_one();
    }

    /// Registers the given addresses with the discovery server.
    ///
    /// `addresses` is the list of reachable local addresses, e.g.
    /// `["tcp://192.168.1.10:22001", "tcp://10.0.0.5:22001"]`.
    ///
    /// # Errors
    /// Returns an error if the request fails or the server responds with a
    /// non-success status.
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

    /// Queries the addresses of the target device.
    ///
    /// Returns an address list like
    /// `["tcp://203.0.113.5:22001", "relay://relay.syncthing.net:22067?id=..."]`,
    /// or an empty list when the device is not registered (404).
    ///
    /// # Errors
    /// Returns an error if the request fails, the response is not JSON, or the
    /// server responds with an error status other than 404.
    pub async fn query(&self, target: DeviceId) -> Result<Vec<String>> {
        let url = format!("{}?device={}", self.server_url, target);

        debug!("GlobalDiscovery query for {} at {}", target, url);

        let resp = self.client.get(&url).send().await.map_err(|e| {
            SyncthingError::network(format!("Global discovery query request failed: {}", e))
        })?;

        let status = resp.status();
        if status.is_success() {
            let data: serde_json::Value = resp.json().await.map_err(|e| {
                SyncthingError::network(format!("Global discovery query JSON parse failed: {}", e))
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
            // Device not registered: an empty list is not an error.
            warn!(
                "Global discovery query for {}: device not found (404)",
                target
            );
            Ok(Vec::new())
        } else {
            let text = resp.text().await.unwrap_or_default();
            error!("Global discovery query failed: HTTP {} - {}", status, text);
            Err(SyncthingError::network(format!(
                "Global discovery query failed: HTTP {} - {}",
                status, text
            )))
        }
    }

    /// Runs the background announce loop.
    ///
    /// Announces immediately (usually with local addresses only), then every
    /// `ANNOUNCE_INTERVAL`. On failure, waits `RETRY_INTERVAL` before the next
    /// attempt. `trigger_reannounce()` wakes the loop for an immediate
    /// re-announce (e.g. after STUN/UPnP discovers new addresses).
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
