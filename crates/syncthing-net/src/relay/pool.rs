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
