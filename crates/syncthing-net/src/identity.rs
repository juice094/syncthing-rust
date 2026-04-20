//! 基于 TLS 证书的设备身份实现
//!
//! `TlsIdentity` 是 `syncthing_core::Identity` 的当前默认实现，
//! 将设备标识与 TLS 证书绑定（SHA-256 证书哈希）。
//!
//! 未来可在此模块中添加 `WireGuardIdentity`、`PreSharedKeyIdentity` 等实现。

use std::sync::Arc;
use syncthing_core::{DeviceId, Identity};

use crate::tls::SyncthingTlsConfig;

/// 基于 TLS 证书的设备身份。
///
/// 设备标识符 = 证书公钥的 SHA-256 哈希。
/// 这是 Syncthing 经典方案，也是当前的默认身份方案。
#[derive(Debug, Clone)]
pub struct TlsIdentity {
    tls_config: Arc<SyncthingTlsConfig>,
}

impl TlsIdentity {
    /// 从 TLS 配置创建身份。
    pub fn new(tls_config: Arc<SyncthingTlsConfig>) -> Self {
        Self { tls_config }
    }

    /// 获取底层 TLS 配置（供传输层握手使用）。
    pub fn tls_config(&self) -> &SyncthingTlsConfig {
        &self.tls_config
    }

    /// 获取 TLS 配置的 `Arc` 引用（供需要 `Arc` 的调用方使用）。
    pub fn tls_config_arc(&self) -> Arc<SyncthingTlsConfig> {
        Arc::clone(&self.tls_config)
    }
}

impl Identity for TlsIdentity {
    fn device_id(&self) -> DeviceId {
        self.tls_config.device_id()
    }

    fn scheme(&self) -> &'static str {
        "tls"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tls_identity_device_id() {
        let temp_dir = std::env::temp_dir().join("syncthing-test-tls-id");
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        let _ = tokio::fs::create_dir_all(&temp_dir).await;

        let tls_config = SyncthingTlsConfig::load_or_generate(&temp_dir)
            .await
            .unwrap();
        let id = TlsIdentity::new(Arc::new(tls_config));

        assert_eq!(id.scheme(), "tls");
        assert_eq!(id.device_id(), id.tls_config().device_id());

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
