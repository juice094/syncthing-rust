//! `syncthing relay-server` — 启动自建 Relay Protocol 服务器
//!
//! 用于替代 Tailscale/Headscale 作为政企内网的 P2P 中继。
//! 与 Go Syncthing relay 服务器完全互操作。

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use syncthing_net::relay::server::RelayServer;
use syncthing_net::tls::{SyncthingTlsConfig, CERT_FILE_NAME, KEY_FILE_NAME};

/// 运行 relay-server 子命令
pub async fn run_relay_server(
    config_dir: &Path,
    listen: SocketAddr,
    session_port: u16,
) -> anyhow::Result<()> {
    let cert_path = config_dir.join(CERT_FILE_NAME);
    let key_path = config_dir.join(KEY_FILE_NAME);

    if !cert_path.exists() || !key_path.exists() {
        anyhow::bail!(
            "TLS certificate not found at {:?} / {:?}.\n\
             修复: 先运行 `syncthing init` 或 `syncthing run` 生成证书",
            cert_path,
            key_path
        );
    }

    // 加载 TLS 证书（与 syncthing 进程共享同一份证书）
    let cert_pem = tokio::fs::read(&cert_path).await?;
    let key_pem = tokio::fs::read(&key_path).await?;
    let tls_config = Arc::new(
        SyncthingTlsConfig::from_pem(&cert_pem, &key_pem)
            .map_err(|e| anyhow::anyhow!("failed to load TLS config: {}", e))?,
    );

    let session_addr = SocketAddr::new(listen.ip(), session_port);

    let server = Arc::new(RelayServer::new(listen, session_addr, tls_config));

    // 注册 Ctrl+C 优雅关闭
    let server_clone = Arc::clone(&server);
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Shutting down relay server...");
        server_clone.shutdown();
    });

    server
        .run()
        .await
        .map_err(|e| anyhow::anyhow!("relay server error: {}", e))
}
