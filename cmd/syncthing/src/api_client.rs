//! 共享 REST API 客户端 — CLI 查询命令 (status, devices, folders) 共用。
//! TUI 不通过 REST 通信（使用 in-process SyncService channel），不在此模块范围。

use anyhow::Context;
use serde::de::DeserializeOwned;

/// 共享 API 客户端，封装 reqwest 和认证逻辑。
pub struct ApiClient {
    client: reqwest::Client,
    api_base: String,
    api_key: String,
}

impl ApiClient {
    /// 从 Config 创建客户端。调用方负责先加载 config。
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

    /// GET 请求并反序列化为 T
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

    /// GET 请求，不解析响应体（用于 health check）
    pub async fn get_raw(&self, path: &str) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}{}", self.api_base, path);
        self.client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {}", url))
    }
}

/// 从 bind address 提取端口，替换 host 为 127.0.0.1
fn api_bind_to_localhost(addr: &str) -> String {
    let port = addr.rsplit(':').next().unwrap_or("8385");
    format!("127.0.0.1:{}", port)
}
