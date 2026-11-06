//! Shared REST API client used by the CLI query commands (status, devices, folders).
//! The TUI does not use REST (it talks to an in-process SyncService) and is out of scope here.

use anyhow::Context;
use serde::de::DeserializeOwned;

/// Shared API client wrapping reqwest and authentication.
pub struct ApiClient {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
}

impl ApiClient {
    /// Creates a client from a loaded config.
    pub fn new(config: &syncthing_core::types::Config) -> Self {
        let api_base = format!("http://{}", api_bind_to_localhost(&config.gui.address));
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("build HTTP client");
        Self {
            client,
            api_base,
            api_key: config.gui.api_key.clone(),
        }
    }

    /// GETs `path` and deserializes the JSON body into `T`.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .client
            .get(&url)
            .header("X-API-Key", &self.api_key)
            .send()
            .await
            .with_context(|| format!("GET {}", url))?;
        resp.json().await.with_context(|| format!("parse {}", url))
    }

    /// GETs `path` without decoding the body (for health checks).
    pub async fn get_raw(&self, path: &str) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}{}", self.api_base, path);
        self.client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {}", url))
    }
}

/// Rebinds a `host:port` bind address to `127.0.0.1:port` for local queries.
fn api_bind_to_localhost(addr: &str) -> String {
    let port = addr.rsplit(':').next().unwrap_or("8385");
    format!("127.0.0.1:{}", port)
}
