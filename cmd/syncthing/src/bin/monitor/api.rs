use tracing::warn;

use crate::sample::ApiSample;

/// Poll the syncthing REST API for connection and folder status.
pub(crate) async fn poll_api(
    client: &reqwest::Client,
    api_addr: &str,
    api_key: &str,
    folder_ids: &[String],
) -> ApiSample {
    let mut sample = ApiSample::default();
    let base = api_addr.trim_end_matches('/');

    // /rest/system/connections
    let connections_url = format!("{}/rest/system/connections", base);
    match client
        .get(&connections_url)
        .header("X-API-Key", api_key)
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => {
                let mut any_connected = false;
                let mut any_direct = false;
                let mut any_relay = false;
                if let Some(conns) = json.get("connections").and_then(|v| v.as_object()) {
                    for (_, info) in conns {
                        let connected = info
                            .get("connected")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if connected {
                            any_connected = true;
                            let address =
                                info.get("address").and_then(|v| v.as_str()).unwrap_or("");
                            if address.starts_with("relay://") || address.contains("relay") {
                                any_relay = true;
                            } else {
                                any_direct = true;
                            }
                        }
                    }
                }
                sample.connected = Some(any_connected);
                sample.connection_type = if any_direct {
                    "direct".to_string()
                } else if any_relay {
                    "relay".to_string()
                } else {
                    String::new()
                };
            }
            Err(e) => {
                warn!("Failed to parse API connections response: {}", e);
            }
        },
        Err(e) => {
            warn!("API connections request failed: {}", e);
        }
    }

    // /rest/db/status for each folder
    let mut total_need_files: u64 = 0;
    let mut total_need_bytes: u64 = 0;
    for folder_id in folder_ids {
        let url = format!("{}/rest/db/status?folder={}", base, folder_id);
        match client.get(&url).header("X-API-Key", api_key).send().await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(json) => {
                    let need_files = json
                        .get("needFiles")
                        .and_then(|v| v.as_u64())
                        .or_else(|| {
                            json.get("needFiles")
                                .and_then(|v| v.as_i64())
                                .map(|v| v as u64)
                        })
                        .unwrap_or(0);
                    let need_bytes = json
                        .get("needBytes")
                        .and_then(|v| v.as_u64())
                        .or_else(|| {
                            json.get("needBytes")
                                .and_then(|v| v.as_i64())
                                .map(|v| v as u64)
                        })
                        .unwrap_or(0);
                    total_need_files += need_files;
                    total_need_bytes += need_bytes;
                    sample
                        .per_folder_need_files
                        .insert(folder_id.clone(), need_files);
                }
                Err(e) => {
                    warn!("Failed to parse db/status for {}: {}", folder_id, e);
                }
            },
            Err(e) => {
                warn!("db/status request for {} failed: {}", folder_id, e);
            }
        }
    }

    if !folder_ids.is_empty() {
        sample.need_files = Some(total_need_files);
        sample.need_bytes = Some(total_need_bytes);
    }

    sample
}
