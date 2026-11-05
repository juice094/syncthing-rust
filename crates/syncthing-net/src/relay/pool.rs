//! Relay Pool client.
//!
//! Fetches the list of available relay servers from the official Syncthing
//! relay pool (`relays.syncthing.net/endpoint`).

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, warn};

use syncthing_core::{Result, SyncthingError};

/// Default relay pool endpoint.
pub const DEFAULT_RELAY_POOL_URL: &str = "https://relays.syncthing.net/endpoint";

/// Single entry in a relay pool response.
#[derive(Debug, serde::Deserialize)]
struct RelayInfo {
    url: String,
}

/// Root structure of a relay pool response.
#[derive(Debug, serde::Deserialize)]
struct RelayPoolResponse {
    relays: Vec<RelayInfo>,
}

/// Fetches the list of available relay addresses from the official relay pool.
///
/// Returns URLs in `relay://host:port/?id=...` format. On request or parse
/// failure returns an error (callers should degrade gracefully).
///
/// # Errors
/// Returns an error if the HTTP client cannot be built, the request fails,
/// or the response is not successful/valid JSON.
pub async fn fetch_relay_pool(endpoint: Option<&str>) -> Result<Vec<String>> {
    let url = endpoint.unwrap_or(DEFAULT_RELAY_POOL_URL);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| SyncthingError::network(format!("relay pool client build: {}", e)))?;

    debug!("Fetching relay pool from {}", url);
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| SyncthingError::network(format!("relay pool request failed: {}", e)))?;

    if !resp.status().is_success() {
        return Err(SyncthingError::network(format!(
            "relay pool returned HTTP {}",
            resp.status()
        )));
    }

    let pool: RelayPoolResponse = resp
        .json()
        .await
        .map_err(|e| SyncthingError::network(format!("relay pool JSON parse failed: {}", e)))?;

    let urls: Vec<String> = pool.relays.into_iter().map(|r| r.url).collect();
    debug!("Relay pool returned {} relay(s)", urls.len());
    Ok(urls)
}

/// Fetches a single default relay address (first entry of the pool).
///
/// For quick-start scenarios: returns `None` when the pool fetch fails, so the
/// caller can fall back to a hardcoded address from configuration.
pub async fn fetch_default_relay() -> Option<String> {
    match fetch_relay_pool(None).await {
        Ok(mut urls) if !urls.is_empty() => urls.pop(),
        Ok(_) => {
            warn!("Relay pool is empty");
            None
        }
        Err(e) => {
            warn!("Failed to fetch relay pool: {}", e);
            None
        }
    }
}

/// TCP-level health check over a relay address list, returning the reachable subset.
///
/// Each address gets a TCP connect attempt with a `timeout_secs` timeout. TLS is
/// not attempted, keeping the check lightweight.
pub async fn filter_healthy_relays(urls: Vec<String>, timeout_secs: u64) -> Vec<String> {
    use super::dial::parse_relay_url;

    let mut healthy = Vec::new();
    for url in urls {
        let (addr, _) = match parse_relay_url(&url) {
            Ok(a) => a,
            Err(e) => {
                debug!("Skipping malformed relay URL {}: {}", url, e);
                continue;
            }
        };
        match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        {
            Ok(Ok(_)) => {
                debug!("Relay {} is healthy", url);
                healthy.push(url);
            }
            Ok(Err(e)) => {
                debug!("Relay {} TCP connect failed: {}", url, e);
            }
            Err(_) => {
                debug!("Relay {} TCP connect timeout", url);
            }
        }
    }
    healthy
}

/// TLS-level health check over a relay address list, returning the reachable subset.
///
/// Per address: parse URL -> TCP connect -> TLS handshake (ALPN = `bep-relay`)
/// -> send `JoinRelayRequest` -> receive success response -> disconnect, all
/// within `timeout_secs`. When `need` > 0, stops after collecting `need`
/// healthy relays to reduce startup latency.
pub async fn filter_healthy_relays_tls(
    urls: Vec<String>,
    timeout_secs: u64,
    tls_config: &crate::tls::SyncthingTlsConfig,
    need: usize,
) -> Vec<String> {
    use super::client::RelayProtocolClient;
    use super::dial::parse_relay_url;

    let rustls_config = match tls_config.relay_client_config() {
        Ok(cfg) => Arc::new(cfg),
        Err(e) => {
            warn!("Failed to build relay TLS config for health check: {}", e);
            return Vec::new();
        }
    };

    let mut healthy = Vec::new();
    for url in urls {
        if need > 0 && healthy.len() >= need {
            break;
        }
        let (addr, _) = match parse_relay_url(&url) {
            Ok(a) => a,
            Err(e) => {
                debug!("Skipping malformed relay URL {}: {}", url, e);
                continue;
            }
        };

        let mut client = match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            RelayProtocolClient::connect(addr, &rustls_config),
        )
        .await
        {
            Ok(Ok(client)) => client,
            Ok(Err(e)) => {
                debug!("Relay {} TLS connect failed: {}", url, e);
                continue;
            }
            Err(_) => {
                debug!("Relay {} TLS connect timeout", url);
                continue;
            }
        };

        let join_result =
            tokio::time::timeout(Duration::from_secs(timeout_secs), client.join_relay()).await;

        // Close the protocol connection explicitly so the relay server does
        // not refuse later listener registrations due to leftover state.
        drop(client);

        match join_result {
            Ok(Ok(())) => {
                debug!("Relay {} passed TLS + JoinRelay health check", url);
                healthy.push(url);
            }
            Ok(Err(e)) => {
                debug!("Relay {} JoinRelay failed: {}", url, e);
            }
            Err(_) => {
                debug!("Relay {} JoinRelay timeout", url);
            }
        }
    }
    healthy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::SyncthingTlsConfig;
    use std::sync::Arc;

    #[test]
    fn test_relay_pool_response_parse() {
        let json = r#"{"relays":[{"url":"relay://192.168.1.1:22067/?id=ABCD"}]}"#;
        let parsed: RelayPoolResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.relays.len(), 1);
        assert_eq!(parsed.relays[0].url, "relay://192.168.1.1:22067/?id=ABCD");
    }

    /// 与 Syncthing 官方 relay pool 进行真实 TLS 健康检查。
    /// 验证本实现能与公共 relay 服务器完成 Protocol Mode TLS + JoinRelay。
    #[tokio::test]
    #[ignore = "requires external network and public relay pool"]
    async fn test_public_relay_pool_tls_health_check() {
        let urls = fetch_relay_pool(None).await.expect("fetch relay pool");
        assert!(!urls.is_empty(), "relay pool should not be empty");

        let tls_config = Arc::new(SyncthingTlsConfig::from_pem(b"", b"").unwrap_or_else(|_| {
            let (cert, key) =
                crate::tls::generate_certificate("relay-pool-test").expect("generate cert");
            SyncthingTlsConfig::from_pem(&cert, &key).expect("load cert")
        }));

        let healthy = filter_healthy_relays_tls(urls, 10, &tls_config, 1).await;
        assert!(
            !healthy.is_empty(),
            "at least one public relay should pass TLS + JoinRelay health check"
        );
    }
}
