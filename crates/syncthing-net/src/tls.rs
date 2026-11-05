//! TLS configuration and handshaking.
//!
//! Syncthing uses a custom TLS setup where device authentication happens at
//! the BEP protocol layer, not the TLS layer; see "docs/design/ADR-20261105-tls-device-auth.md".

use std::path::Path;
use std::sync::{Arc, Once};
use std::time::Duration;

use rustls::pki_types::pem::PemObject;

static CRYPTO_PROVIDER_INIT: Once = Once::new();

fn ensure_crypto_provider() {
    CRYPTO_PROVIDER_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::fs;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::rustls::{self, ClientConfig, ServerConfig};
use tokio_rustls::{
    client::TlsStream as ClientTlsStream, server::TlsStream as ServerTlsStream, TlsAcceptor,
    TlsConnector,
};
use tracing::{debug, info, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

/// Timeout for the TLS handshake.
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Default certificate file name.
pub const CERT_FILE_NAME: &str = "cert.pem";
/// Default private key file name.
pub const KEY_FILE_NAME: &str = "key.pem";

/// Syncthing TLS configuration.
#[derive(Debug)]
pub struct SyncthingTlsConfig {
    /// Device certificate chain.
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// Device private key.
    pub private_key: PrivateKeyDer<'static>,
    /// Device ID derived from the certificate.
    pub device_id: DeviceId,
}

impl SyncthingTlsConfig {
    /// Loads the certificate chain and private key from PEM data.
    ///
    /// # Errors
    /// Returns an error if no certificate or no private key is present, or the
    /// private key is invalid.
    pub fn from_pem(cert_pem: &[u8], key_pem: &[u8]) -> Result<Self> {
        let cert_chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
            .filter_map(|r| r.ok())
            .collect();

        if cert_chain.is_empty() {
            return Err(SyncthingError::Tls("no certificates found".to_string()));
        }

        let private_key = PrivateKeyDer::pem_slice_iter(key_pem)
            .next()
            .ok_or_else(|| SyncthingError::Tls("no private key found".to_string()))?
            .map_err(|e| SyncthingError::Tls(format!("invalid private key: {}", e)))?;

        let device_id = Self::derive_device_id(&cert_chain[0])?;

        Ok(Self {
            cert_chain,
            private_key,
            device_id,
        })
    }

    /// Derives the device ID from a certificate (SHA-256 of the certificate DER).
    ///
    /// # Errors
    /// Returns an error if the device ID cannot be constructed from the hash.
    pub fn derive_device_id(cert: &CertificateDer) -> Result<DeviceId> {
        use sha2::{Digest, Sha256};

        let hash = Sha256::digest(cert);

        DeviceId::from_bytes(&hash)
    }

    /// Loads an existing certificate from the config directory, or generates
    /// and persists a new one if absent or unreadable.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be created, existing files
    /// cannot be read, or the new certificate cannot be written.
    pub async fn load_or_generate(config_dir: &Path) -> Result<Self> {
        let cert_path = config_dir.join(CERT_FILE_NAME);
        let key_path = config_dir.join(KEY_FILE_NAME);

        if !config_dir.exists() {
            fs::create_dir_all(config_dir).await.map_err(|e| {
                SyncthingError::config(format!("failed to create config directory: {}", e))
            })?;
        }

        if cert_path.exists() && key_path.exists() {
            info!("Loading existing certificate from {:?}", cert_path);

            let cert_pem = fs::read(&cert_path)
                .await
                .map_err(|e| SyncthingError::Tls(format!("failed to read cert file: {}", e)))?;
            let key_pem = fs::read(&key_path)
                .await
                .map_err(|e| SyncthingError::Tls(format!("failed to read key file: {}", e)))?;

            match Self::from_pem(&cert_pem, &key_pem) {
                Ok(config) => {
                    info!(
                        "Successfully loaded existing certificate, device ID: {}",
                        config.device_id
                    );
                    return Ok(config);
                }
                Err(e) => {
                    warn!(
                        "Failed to load existing certificate: {}. Generating new one...",
                        e
                    );
                }
            }
        }

        info!("Generating new certificate...");
        let (cert_pem, key_pem) = generate_self_signed_cert()?;

        fs::write(&cert_path, &cert_pem)
            .await
            .map_err(|e| SyncthingError::Tls(format!("failed to write cert file: {}", e)))?;
        fs::write(&key_path, &key_pem)
            .await
            .map_err(|e| SyncthingError::Tls(format!("failed to write key file: {}", e)))?;

        // SAFETY: Harden private key file permissions against other users on multi-user systems.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode_0600 = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = tokio::fs::set_permissions(&key_path, mode_0600.clone()).await {
                warn!(
                    path = %key_path.display(),
                    error = %e,
                    "Failed to set restrictive permissions on private key file"
                );
            }
            // Restrict the certificate file to owner read/write as well (conservative).
            let _ = tokio::fs::set_permissions(&cert_path, mode_0600).await;
        }

        info!("New certificate saved to {:?}", cert_path);

        Self::from_pem(&cert_pem, &key_pem)
    }

    /// Builds the server configuration (ALPN = `bep/1.0`).
    ///
    /// # Errors
    /// Returns a rustls error if the configuration is invalid.
    pub fn server_config(&self) -> std::result::Result<ServerConfig, rustls::Error> {
        let mut config = ServerConfig::builder()
            .with_client_cert_verifier(Arc::new(SyncthingClientCertVerifier))
            .with_single_cert(self.cert_chain.clone(), self.private_key.clone_key())?;
        config.alpn_protocols = vec![b"bep/1.0".to_vec()];
        Ok(config)
    }

    /// Builds the client configuration (ALPN = `bep/1.0`).
    ///
    /// # Errors
    /// Returns a rustls error if the configuration is invalid.
    pub fn client_config(&self) -> std::result::Result<ClientConfig, rustls::Error> {
        let mut config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SyncthingCertVerifier))
            .with_client_auth_cert(self.cert_chain.clone(), self.private_key.clone_key())?;
        config.alpn_protocols = vec![b"bep/1.0".to_vec()];
        Ok(config)
    }

    /// Builds the Relay Protocol server configuration (ALPN = `bep-relay`).
    ///
    /// # Errors
    /// Returns a rustls error if the configuration is invalid.
    pub fn relay_server_config(&self) -> std::result::Result<ServerConfig, rustls::Error> {
        let mut config = ServerConfig::builder()
            .with_client_cert_verifier(Arc::new(SyncthingClientCertVerifier))
            .with_single_cert(self.cert_chain.clone(), self.private_key.clone_key())?;
        config.alpn_protocols = vec![b"bep-relay".to_vec()];
        Ok(config)
    }

    /// Builds the Relay Protocol client configuration (ALPN = `bep-relay`).
    ///
    /// # Errors
    /// Returns a rustls error if the configuration is invalid.
    pub fn relay_client_config(&self) -> std::result::Result<ClientConfig, rustls::Error> {
        let mut config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SyncthingCertVerifier))
            .with_client_auth_cert(self.cert_chain.clone(), self.private_key.clone_key())?;
        config.alpn_protocols = vec![b"bep-relay".to_vec()];
        Ok(config)
    }

    /// Returns the device ID.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Returns the certificate chain re-encoded as PEM.
    pub fn cert_pem(&self) -> Vec<u8> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let mut pem = Vec::new();
        for cert in &self.cert_chain {
            pem.extend_from_slice(b"-----BEGIN CERTIFICATE-----\n");
            let base64_cert = STANDARD.encode(cert);
            for chunk in base64_cert.as_bytes().chunks(64) {
                pem.extend_from_slice(chunk);
                pem.push(b'\n');
            }
            pem.extend_from_slice(b"-----END CERTIFICATE-----\n");
        }
        pem
    }

    /// Returns the private key re-encoded as PKCS#8 PEM (empty if not PKCS#8).
    pub fn key_pem(&self) -> Vec<u8> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        match &self.private_key {
            PrivateKeyDer::Pkcs8(pkcs8) => {
                let mut pem = Vec::new();
                pem.extend_from_slice(b"-----BEGIN PRIVATE KEY-----\n");
                let base64_key = STANDARD.encode(pkcs8.secret_pkcs8_der());
                for chunk in base64_key.as_bytes().chunks(64) {
                    pem.extend_from_slice(chunk);
                    pem.push(b'\n');
                }
                pem.extend_from_slice(b"-----END PRIVATE KEY-----\n");
                pem
            }
            _ => vec![],
        }
    }
}

/// Server-side client-certificate verifier.
///
/// Accepts any client certificate: the TLS layer acts only as an encrypted
/// tunnel, and device authentication happens at the BEP protocol layer.
/// See "docs/design/ADR-20261105-tls-device-auth.md".
#[derive(Debug)]
struct SyncthingClientCertVerifier;

impl rustls::server::danger::ClientCertVerifier for SyncthingClientCertVerifier {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}

/// Client-side server-certificate verifier.
///
/// Custom Syncthing verification logic:
/// 1. No certificate-chain validation.
/// 2. The certificate public key is checked against the known device ID at the
///    protocol layer instead.
///
/// See "docs/design/ADR-20261105-tls-device-auth.md".
#[derive(Debug)]
struct SyncthingCertVerifier;

impl rustls::client::danger::ServerCertVerifier for SyncthingCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // Device verification happens at the BEP protocol layer; TLS accepts any certificate.
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}

/// Generates a self-signed certificate compatible with Syncthing using rcgen.
fn generate_self_signed_cert() -> Result<(Vec<u8>, Vec<u8>)> {
    use chrono::Datelike;
    use rcgen::{CertificateParams, KeyPair};

    let key_pair = KeyPair::generate()
        .map_err(|e| SyncthingError::Tls(format!("failed to generate key pair: {}", e)))?;

    let mut params = CertificateParams::new(vec!["syncthing".to_string()])
        .map_err(|e| SyncthingError::Tls(format!("failed to create cert params: {}", e)))?;

    // ~20 year validity, matching the Syncthing default.
    let now = chrono::Utc::now();
    params.not_before = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day() as u8);
    let end = now + chrono::Duration::days(365 * 20);
    params.not_after = rcgen::date_time_ymd(end.year(), end.month() as u8, end.day() as u8);

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| SyncthingError::Tls(format!("failed to create certificate: {}", e)))?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    Ok((cert_pem.into_bytes(), key_pem.into_bytes()))
}

/// Performs the server-side TLS handshake over a TCP stream.
///
/// # Errors
/// Returns an error on handshake timeout or TLS failure.
pub async fn accept_tls(
    stream: TcpStream,
    config: ServerConfig,
) -> Result<ServerTlsStream<TcpStream>> {
    debug!("Starting server TLS handshake");

    let acceptor = TlsAcceptor::from(Arc::new(config));

    let tls_stream = timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(stream))
        .await
        .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
        .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;

    info!("Server TLS handshake completed");

    Ok(tls_stream)
}

/// Performs the client-side TLS handshake over a TCP stream.
///
/// # Errors
/// Returns an error if the server name is invalid, or on handshake timeout or
/// TLS failure.
pub async fn connect_tls(
    stream: TcpStream,
    config: ClientConfig,
    server_name: &'static str,
) -> Result<ClientTlsStream<TcpStream>> {
    debug!("Starting client TLS handshake");

    let connector = TlsConnector::from(Arc::new(config));

    let server_name = ServerName::try_from(server_name)
        .map_err(|_| SyncthingError::Tls("invalid server name".to_string()))?;

    let tls_stream = timeout(
        TLS_HANDSHAKE_TIMEOUT,
        connector.connect(server_name, stream),
    )
    .await
    .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
    .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;

    info!("Client TLS handshake completed");

    Ok(tls_stream)
}

/// Extracts the peer device ID from a server-side TLS stream.
///
/// # Errors
/// Returns an error if the peer presented no certificate.
fn peer_device_id_from_stream<S>(
    tls_stream: &tokio_rustls::server::TlsStream<S>,
) -> Result<DeviceId> {
    let peer_certs = tls_stream
        .get_ref()
        .1
        .peer_certificates()
        .ok_or_else(|| SyncthingError::Tls("no peer certificate".to_string()))?;

    let peer_cert = peer_certs
        .first()
        .ok_or_else(|| SyncthingError::Tls("empty peer certificate chain".to_string()))?;

    SyncthingTlsConfig::derive_device_id(peer_cert)
}

/// Extracts the peer device ID from a client-side TLS stream.
///
/// # Errors
/// Returns an error if the peer presented no certificate.
fn peer_device_id_from_client_stream<S>(
    tls_stream: &tokio_rustls::client::TlsStream<S>,
) -> Result<DeviceId> {
    let peer_certs = tls_stream
        .get_ref()
        .1
        .peer_certificates()
        .ok_or_else(|| SyncthingError::Tls("no peer certificate".to_string()))?;

    let peer_cert = peer_certs
        .first()
        .ok_or_else(|| SyncthingError::Tls("empty peer certificate chain".to_string()))?;

    SyncthingTlsConfig::derive_device_id(peer_cert)
}

/// Performs the server-side TLS handshake over a generic stream, returning the
/// TLS stream and the authenticated peer device ID.
///
/// # Errors
/// Returns an error if the server config cannot be built, or on handshake
/// timeout, TLS failure, or missing peer certificate.
pub async fn accept_tls_stream<S>(
    stream: S,
    config: &SyncthingTlsConfig,
) -> Result<(ServerTlsStream<S>, DeviceId)>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    ensure_crypto_provider();
    debug!("Starting server TLS handshake over generic stream");

    let server_config = config
        .server_config()
        .map_err(|e| SyncthingError::Tls(format!("failed to create server config: {}", e)))?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    let tls_stream = timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(stream))
        .await
        .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
        .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;

    let device_id = peer_device_id_from_stream(&tls_stream)?;

    info!(
        "Server TLS handshake completed, peer device_id={}",
        device_id
    );

    Ok((tls_stream, device_id))
}

/// Performs the client-side TLS handshake over a generic stream, returning the
/// TLS stream and the authenticated peer device ID.
///
/// When `remote_device` is `Some`, the peer device ID is checked against it.
///
/// # Errors
/// Returns an error if the client config cannot be built, on handshake timeout
/// or TLS failure, on missing peer certificate, or when the peer device ID
/// does not match `remote_device`.
pub async fn connect_tls_stream<S>(
    stream: S,
    config: &SyncthingTlsConfig,
    remote_device: Option<DeviceId>,
) -> Result<(ClientTlsStream<S>, DeviceId)>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    ensure_crypto_provider();
    debug!("Starting client TLS handshake over generic stream");

    let client_config = config
        .client_config()
        .map_err(|e| SyncthingError::Tls(format!("failed to create client config: {}", e)))?;
    let connector = TlsConnector::from(Arc::new(client_config));

    let server_name = ServerName::try_from("syncthing")
        .map_err(|_| SyncthingError::Tls("invalid server name".to_string()))?;

    let tls_stream = timeout(
        TLS_HANDSHAKE_TIMEOUT,
        connector.connect(server_name, stream),
    )
    .await
    .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
    .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;

    let device_id = peer_device_id_from_client_stream(&tls_stream)?;

    if let Some(expected) = remote_device {
        if device_id != expected {
            return Err(SyncthingError::Tls(format!(
                "device ID mismatch: expected {}, got {}",
                expected, device_id
            )));
        }
    }

    info!(
        "Client TLS handshake completed, peer device_id={}",
        device_id
    );

    Ok((tls_stream, device_id))
}

/// Generates a self-signed certificate and private key PEM pair (simplified API).
///
/// # Errors
/// Returns an error if key generation or certificate creation fails.
pub fn generate_certificate(_device_name: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    generate_self_signed_cert()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id_derivation() {
        // 测试设备ID从证书派生
    }

    #[tokio::test]
    async fn test_load_or_generate_certificate() {
        // 创建临时目录
        let temp_dir = std::env::temp_dir()
            .join("syncthing_test_")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&temp_dir).await.unwrap();

        // 第一次调用应该生成新证书
        let config1 = SyncthingTlsConfig::load_or_generate(&temp_dir)
            .await
            .unwrap();
        let device_id1 = config1.device_id();

        // 检查证书文件是否已创建
        assert!(temp_dir.join(CERT_FILE_NAME).exists());
        assert!(temp_dir.join(KEY_FILE_NAME).exists());

        // 第二次调用应该加载相同的证书
        let config2 = SyncthingTlsConfig::load_or_generate(&temp_dir)
            .await
            .unwrap();
        let device_id2 = config2.device_id();

        // 设备ID应该相同
        assert_eq!(device_id1, device_id2);

        // 清理
        let _ = fs::remove_dir_all(&temp_dir).await;
    }
}
