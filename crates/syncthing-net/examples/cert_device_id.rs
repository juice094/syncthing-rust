//! 示例：从 Syncthing 配置目录加载（或生成）证书，并打印对应的 Device ID。
//!
//! 用法：
//! ```bash
//! cargo run --example cert_device_id -p syncthing-net -- ~/.syncthing
//! ```
//!
//! 如果目录中不存在 `cert.pem`/`key.pem`，会生成新的 Ed25519 证书并持久化。

use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cert_device_id <config-dir>");

    let tls = syncthing_net::tls::SyncthingTlsConfig::load_or_generate(&dir).await?;
    println!("{}", tls.device_id());
    Ok(())
}
