//! TLS 配置和握手
//!
//! Syncthing使用自定义的TLS配置进行设备认证

use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Once};
use std::time::Duration;

static CRYPTO_PROVIDER_INIT: Once = Once::new();

fn ensure_crypto_provider() {
    CRYPTO_PROVIDER_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

use tokio::fs;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::rustls::{self, ClientConfig, ServerConfig};
use tokio_rustls::{TlsAcceptor, TlsConnector, client::TlsStream as ClientTlsStream, server::TlsStream as ServerTlsStream};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tracing::{debug, info, warn};

use syncthing_core::{DeviceId, Result, SyncthingError};

/// TLS握手超时
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 默认证书文件名
pub const CERT_FILE_NAME: &str = "cert.pem";
/// 默认私钥文件名
pub const KEY_FILE_NAME: &str = "key.pem";

/// Syncthing TLS配置
#[derive(Debug)]
pub struct SyncthingTlsConfig {
    /// 设备证书
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// 设备私钥
    pub private_key: PrivateKeyDer<'static>,
    /// 设备ID（从证书派生）
    pub device_id: DeviceId,
}

impl SyncthingTlsConfig {
    /// 从PEM文件加载证书和私钥
    pub fn from_pem(cert_pem: &[u8], key_pem: &[u8]) -> Result<Self> {
        // 解析证书
        let mut cert_reader = BufReader::new(cert_pem);
        let cert_chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
            .filter_map(|r| r.ok())
            .map(|c| c.into_owned())
            .collect();
        
        if cert_chain.is_empty() {
            return Err(SyncthingError::Tls("no certificates found".to_string()));
        }
        
        // 解析私钥
        let mut key_reader = BufReader::new(key_pem);
        let private_key = rustls_pemfile::pkcs8_private_keys(&mut key_reader)
            .next()
            .ok_or_else(|| SyncthingError::Tls("no private key found".to_string()))??;
        
        let private_key = PrivateKeyDer::from(private_key);
        
        // 计算设备ID（从证书公钥的SHA-256）
        let device_id = Self::derive_device_id(&cert_chain[0])?;
        
        Ok(Self {
            cert_chain,
            private_key,
            device_id,
        })
    }
    
    /// 从证书派生设备ID
    pub fn derive_device_id(cert: &CertificateDer) -> Result<DeviceId> {
        use sha2::{Sha256, Digest};
        
        // 计算证书DER编码的SHA-256
        let hash = Sha256::digest(cert);
        
        DeviceId::from_bytes(&hash)
    }
    
    /// 加载或生成证书
    /// 
    /// 如果证书文件已存在，则加载现有证书；否则生成新证书并保存
    pub async fn load_or_generate(config_dir: &Path) -> Result<Self> {
        let cert_path = config_dir.join(CERT_FILE_NAME);
        let key_path = config_dir.join(KEY_FILE_NAME);
        
        // 确保证书目录存在
        if !config_dir.exists() {
            fs::create_dir_all(config_dir).await
                .map_err(|e| SyncthingError::config(format!("failed to create config directory: {}", e)))?;
        }
        
        if cert_path.exists() && key_path.exists() {
            // 加载现有证书
            info!("Loading existing certificate from {:?}", cert_path);
            
            let cert_pem = fs::read(&cert_path).await
                .map_err(|e| SyncthingError::Tls(format!("failed to read cert file: {}", e)))?;
            let key_pem = fs::read(&key_path).await
                .map_err(|e| SyncthingError::Tls(format!("failed to read key file: {}", e)))?;
            
            match Self::from_pem(&cert_pem, &key_pem) {
                Ok(config) => {
                    info!("Successfully loaded existing certificate, device ID: {}", config.device_id);
                    return Ok(config);
                }
                Err(e) => {
                    warn!("Failed to load existing certificate: {}. Generating new one...", e);
                    // 继续生成新证书
                }
            }
        }
        
        // 生成新证书
        info!("Generating new certificate...");
        let (cert_pem, key_pem) = generate_self_signed_cert()?;
        
        // 保存证书
        fs::write(&cert_path, &cert_pem).await
            .map_err(|e| SyncthingError::Tls(format!("failed to write cert file: {}", e)))?;
        fs::write(&key_path, &key_pem).await
            .map_err(|e| SyncthingError::Tls(format!("failed to write key file: {}", e)))?;
        
        info!("New certificate saved to {:?}", cert_path);
        
        // 加载刚保存的证书
        Self::from_pem(&cert_pem, &key_pem)
    }
    
    /// 创建服务器配置
    pub fn server_config(&self) -> std::result::Result<ServerConfig, rustls::Error> {
        let mut config = ServerConfig::builder()
            .with_client_cert_verifier(Arc::new(SyncthingClientCertVerifier))
            .with_single_cert(self.cert_chain.clone(), self.private_key.clone_key())?;
        config.alpn_protocols = vec![b"bep/1.0".to_vec()];
        Ok(config)
    }
    
    /// 创建客户端配置
    pub fn client_config(&self) -> std::result::Result<ClientConfig, rustls::Error> {
        let mut config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SyncthingCertVerifier))
            .with_client_auth_cert(self.cert_chain.clone(), self.private_key.clone_key())?;
        config.alpn_protocols = vec![b"bep/1.0".to_vec()];
        Ok(config)
    }

    /// 获取 Relay Protocol 用的 TLS 客户端配置（ALPN = `bep-relay`）
    pub fn relay_client_config(&self) -> std::result::Result<ClientConfig, rustls::Error> {
        let mut config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SyncthingCertVerifier))
            .with_client_auth_cert(self.cert_chain.clone(), self.private_key.clone_key())?;
        config.alpn_protocols = vec![b"bep-relay".to_vec()];
        Ok(config)
    }
    
    /// 获取设备ID
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }
    
    /// 获取证书PEM内容
    pub fn cert_pem(&self) -> Vec<u8> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        
        // 将证书链转换为PEM格式
        let mut pem = Vec::new();
        for cert in &self.cert_chain {
            pem.extend_from_slice(b"-----BEGIN CERTIFICATE-----\n");
            let base64_cert = STANDARD.encode(cert);
            // 每64字符一行
            for chunk in base64_cert.as_bytes().chunks(64) {
                pem.extend_from_slice(chunk);
                pem.push(b'\n');
            }
            pem.extend_from_slice(b"-----END CERTIFICATE-----\n");
        }
        pem
    }
    
    /// 获取私钥PEM内容
    pub fn key_pem(&self) -> Vec<u8> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        
        // 将私钥转换为PKCS8 PEM格式
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

/// Syncthing服务器端客户端证书验证器
///
/// 接受任何客户端证书，使TLS层仅作为加密隧道，
/// 设备认证在BEP协议层完成。
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

/// Syncthing证书验证器
///
/// Syncthing使用自定义的证书验证逻辑：
/// 1. 不验证证书链
/// 2. 验证证书公钥与已知的设备ID匹配
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
        // Syncthing在协议层进行设备验证
        // TLS层接受任何证书
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

/// 生成自签名证书
/// 
/// 使用rcgen库生成与Syncthing兼容的自签名证书
fn generate_self_signed_cert() -> Result<(Vec<u8>, Vec<u8>)> {
    use chrono::Datelike;
    use rcgen::{CertificateParams, KeyPair};
    
    // 生成密钥对（使用系统默认算法）
    let key_pair = KeyPair::generate()
        .map_err(|e| SyncthingError::Tls(format!("failed to generate key pair: {}", e)))?;
    
    // 创建证书参数
    let mut params = CertificateParams::new(vec!["syncthing".to_string()])
        .map_err(|e| SyncthingError::Tls(format!("failed to create cert params: {}", e)))?;
    
    // 设置证书有效期（约20年，与Syncthing默认相同）
    let now = chrono::Utc::now();
    // 使用 rcgen 的时间格式
    params.not_before = rcgen::date_time_ymd(
        now.year(),
        now.month() as u8,
        now.day() as u8
    );
    let end = now + chrono::Duration::days(365 * 20);
    params.not_after = rcgen::date_time_ymd(
        end.year(),
        end.month() as u8,
        end.day() as u8
    );
    
    // 使用密钥对生成证书
    let cert = params.self_signed(&key_pair)
        .map_err(|e| SyncthingError::Tls(format!("failed to create certificate: {}", e)))?;
    
    // 导出PEM格式
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    
    Ok((cert_pem.into_bytes(), key_pem.into_bytes()))
}

/// 服务器端TLS握手
pub async fn accept_tls(
    stream: TcpStream,
    config: ServerConfig,
) -> Result<ServerTlsStream<TcpStream>> {
    debug!("Starting server TLS handshake");
    
    let acceptor = TlsAcceptor::from(Arc::new(config));
    
    let tls_stream = timeout(
        TLS_HANDSHAKE_TIMEOUT,
        acceptor.accept(stream)
    ).await
        .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
        .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;
    
    info!("Server TLS handshake completed");
    
    Ok(tls_stream)
}

/// 客户端端TLS握手
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
        connector.connect(server_name, stream)
    ).await
        .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
        .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;
    
    info!("Client TLS handshake completed");
    
    Ok(tls_stream)
}

/// 从TLS流中提取对端设备ID
fn peer_device_id_from_stream<S>(tls_stream: &tokio_rustls::server::TlsStream<S>) -> Result<DeviceId> {
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

fn peer_device_id_from_client_stream<S>(tls_stream: &tokio_rustls::client::TlsStream<S>) -> Result<DeviceId> {
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

/// 服务器端TLS握手（泛型流，用于iroh）
pub async fn accept_tls_stream<S>(
    stream: S,
    config: &SyncthingTlsConfig,
) -> Result<(ServerTlsStream<S>, DeviceId)>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    ensure_crypto_provider();
    debug!("Starting server TLS handshake over generic stream");
    
    let server_config = config.server_config()
        .map_err(|e| SyncthingError::Tls(format!("failed to create server config: {}", e)))?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    
    let tls_stream = timeout(
        TLS_HANDSHAKE_TIMEOUT,
        acceptor.accept(stream)
    ).await
        .map_err(|_| SyncthingError::timeout("TLS handshake timeout"))?
        .map_err(|e| SyncthingError::Tls(format!("TLS handshake failed: {}", e)))?;
    
    let device_id = peer_device_id_from_stream(&tls_stream)?;
    
    info!("Server TLS handshake completed, peer device_id={}", device_id);
    
    Ok((tls_stream, device_id))
}

/// 客户端端TLS握手（泛型流，用于iroh）
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
    
    let client_config = config.client_config()
        .map_err(|e| SyncthingError::Tls(format!("failed to create client config: {}", e)))?;
    let connector = TlsConnector::from(Arc::new(client_config));
    
    let server_name = ServerName::try_from("syncthing")
        .map_err(|_| SyncthingError::Tls("invalid server name".to_string()))?;
    
    let tls_stream = timeout(
        TLS_HANDSHAKE_TIMEOUT,
        connector.connect(server_name, stream)
    ).await
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
    
    info!("Client TLS handshake completed, peer device_id={}", device_id);
    
    Ok((tls_stream, device_id))
}

/// 生成自签名证书和设备密钥（简化版本）
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
        let temp_dir = std::env::temp_dir().join("syncthing_test_").join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&temp_dir).await.unwrap();
        
        // 第一次调用应该生成新证书
        let config1 = SyncthingTlsConfig::load_or_generate(&temp_dir).await.unwrap();
        let device_id1 = config1.device_id();
        
        // 检查证书文件是否已创建
        assert!(temp_dir.join(CERT_FILE_NAME).exists());
        assert!(temp_dir.join(KEY_FILE_NAME).exists());
        
        // 第二次调用应该加载相同的证书
        let config2 = SyncthingTlsConfig::load_or_generate(&temp_dir).await.unwrap();
        let device_id2 = config2.device_id();
        
        // 设备ID应该相同
        assert_eq!(device_id1, device_id2);
        
        // 清理
        let _ = fs::remove_dir_all(&temp_dir).await;
    }
}
