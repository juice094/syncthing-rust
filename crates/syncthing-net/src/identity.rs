//! TLS-certificate-based device identity.
//!
//! `TlsIdentity` is the default implementation of `syncthing_core::Identity`,
//! binding the device identifier to a TLS certificate (SHA-256 certificate
//! hash). Future identity schemes (e.g. WireGuard or pre-shared keys) would
//! implement the same `Identity` trait in this module.

use std::sync::Arc;
use syncthing_core::{DeviceId, Identity};

use crate::tls::SyncthingTlsConfig;

/// Device identity backed by a TLS certificate.
///
/// The device identifier is the SHA-256 hash of the certificate public key.
/// This is the classic Syncthing scheme and the current default identity.
#[derive(Debug, Clone)]
pub struct TlsIdentity {
    tls_config: Arc<SyncthingTlsConfig>,
}

impl TlsIdentity {
    /// Creates an identity from a TLS configuration.
    pub fn new(tls_config: Arc<SyncthingTlsConfig>) -> Self {
        Self { tls_config }
    }

    /// Returns the underlying TLS configuration (used by transport handshakes).
    pub fn tls_config(&self) -> &SyncthingTlsConfig {
        &self.tls_config
    }

    /// Returns a clone of the `Arc` holding the TLS configuration.
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
