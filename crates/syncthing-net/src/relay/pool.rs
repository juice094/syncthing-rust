//! Relay Pool 客户端
//!
//! 从 Syncthing 官方 relay pool (`relays.syncthing.net/endpoint`) 获取可用中继服务器列表。

use std::time::Duration;

use tracing::{debug, warn};

use syncthing_core::{Result, SyncthingError};

/// 默认 relay pool endpoint
pub const DEFAULT_RELAY_POOL_URL: &str = "https://relays.syncthing.net/endpoint";

/// Relay pool 响应中的单条记录
#[derive(Debug, serde::Deserialize)]
struct RelayInfo {
    url: String,
}

/// Relay pool 响应根结构
#[derive(Debug, serde::Deserialize)]
struct RelayPoolResponse {
    relays: Vec<RelayInfo>,
}

/// 从官方 relay pool 获取可用 relay 地址列表
///
/// # 返回
/// `relay://host:port/?id=...` 格式的 URL 列表。
/// 若请求失败或解析失败，返回错误（调用方应优雅降级）。
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

    let pool: RelayPoolResponse = resp.json().await.map_err(|e| {
        SyncthingError::network(format!("relay pool JSON parse failed: {}", e))
    })?;

    let urls: Vec<String> = pool.relays.into_iter().map(|r| r.url).collect();
    debug!("Relay pool returned {} relay(s)", urls.len());
    Ok(urls)
}

/// 获取单个默认 relay 地址（从 pool 中取第一个）
///
/// 适合快速启动场景：若 pool 获取失败，返回 `None`，调用方可回退到配置中的硬编码地址。
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

/// 对 relay 地址列表进行 TCP 健康检查，返回可达的子集
///
/// 每个地址尝试 TCP connect，超时 `timeout_secs` 秒。
/// 仅检查 TCP 层连通性，不完成 TLS 握手，避免过重开销。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_pool_response_parse() {
        let json = r#"{"relays":[{"url":"relay://192.168.1.1:22067/?id=ABCD"}]}"#;
        let parsed: RelayPoolResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.relays.len(), 1);
        assert_eq!(parsed.relays[0].url, "relay://192.168.1.1:22067/?id=ABCD");
    }
}
