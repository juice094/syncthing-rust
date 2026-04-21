//! 代理传输层
//!
//! 支持通过 HTTP CONNECT 和 SOCKS5 代理建立出站连接，
//! 用于突破企业/校园网的出站限制。
//!
//! 代理配置优先从环境变量读取：
//! - `HTTP_PROXY` / `http_proxy`
//! - `SOCKS5_PROXY` / `socks5_proxy` / `ALL_PROXY`
//!
//! 未来扩展：SOCKS5 完整实现、代理认证、代理链。

use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use syncthing_core::{
    BoxedPipe, Result, SyncthingError, Transport, TransportListener, TransportType,
};
use syncthing_core::traits::ReliablePipe;

/// 代理配置
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// 代理类型
    pub proxy_type: ProxyType,
    /// 代理服务器地址
    pub addr: SocketAddr,
    /// 认证信息（可选）
    pub auth: Option<ProxyAuth>,
}

#[derive(Debug, Clone)]
pub enum ProxyType {
    HttpConnect,
    Socks5,
}

#[derive(Debug, Clone)]
pub struct ProxyAuth {
    pub username: String,
    pub password: String,
}

impl ProxyConfig {
    /// 从环境变量读取代理配置
    ///
    /// 优先级：SOCKS5_PROXY > ALL_PROXY > HTTP_PROXY
    pub fn from_env() -> Option<Self> {
        let socks = std::env::var("SOCKS5_PROXY")
            .or_else(|_| std::env::var("socks5_proxy"))
            .or_else(|_| std::env::var("ALL_PROXY"))
            .or_else(|_| std::env::var("all_proxy"));

        if let Ok(url) = socks {
            if let Some(cfg) = Self::parse_socks_url(&url) {
                return Some(cfg);
            }
        }

        let http = std::env::var("HTTP_PROXY")
            .or_else(|_| std::env::var("http_proxy"));

        if let Ok(url) = http {
            if let Some(cfg) = Self::parse_http_url(&url) {
                return Some(cfg);
            }
        }

        None
    }

    fn parse_socks_url(url: &str) -> Option<Self> {
        // 简单解析：socks5://host:port 或 host:port
        let addr_str = url.strip_prefix("socks5://").unwrap_or(url);
        addr_str.parse().ok().map(|addr| Self {
            proxy_type: ProxyType::Socks5,
            addr,
            auth: None,
        })
    }

    fn parse_http_url(url: &str) -> Option<Self> {
        // 简单解析：http://host:port 或 host:port
        let addr_str = url.strip_prefix("http://").unwrap_or(url);
        addr_str.parse().ok().map(|addr| Self {
            proxy_type: ProxyType::HttpConnect,
            addr,
            auth: None,
        })
    }
}

/// 代理传输层。
///
/// 包装底层 TCP 连接，通过代理服务器建立到目标的隧道。
/// 当前支持 HTTP CONNECT；SOCKS5 为 stub（返回错误）。
#[derive(Debug)]
pub struct ProxiedTransport {
    config: ProxyConfig,
}

impl ProxiedTransport {
    pub fn new(config: ProxyConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Option<Self> {
        ProxyConfig::from_env().map(Self::new)
    }
}

#[async_trait::async_trait]
impl Transport for ProxiedTransport {
    fn scheme(&self) -> &'static str {
        match self.config.proxy_type {
            ProxyType::HttpConnect => "http-proxy",
            ProxyType::Socks5 => "socks5-proxy",
        }
    }

    async fn bind(&self, _addr: SocketAddr) -> Result<Box<dyn TransportListener>> {
        Err(SyncthingError::config(
            "ProxiedTransport does not support inbound listening"
        ))
    }

    async fn dial(&self, target: SocketAddr) -> Result<BoxedPipe> {
        info!("Dialing {} via {:?} proxy at {}", target, self.config.proxy_type, self.config.addr);

        // 1. 连接到代理服务器
        let proxy_stream = TcpStream::connect(self.config.addr).await.map_err(|e| {
            SyncthingError::connection(format!("proxy connect failed: {}", e))
        })?;

        // 2. 根据代理类型建立隧道
        match self.config.proxy_type {
            ProxyType::HttpConnect => {
                let stream = http_connect_handshake(proxy_stream, target).await?;
                Ok(Box::new(ProxyPipe {
                    stream,
                    local_addr: None,
                    peer_addr: Some(target),
                }))
            }
            ProxyType::Socks5 => {
                warn!("SOCKS5 proxy is not yet fully implemented");
                Err(SyncthingError::connection(
                    "SOCKS5 proxy not yet implemented"
                ))
            }
        }
    }
}

/// HTTP CONNECT 握手
async fn http_connect_handshake(
    mut stream: TcpStream,
    target: SocketAddr,
) -> Result<TcpStream> {
    let request = format!(
        "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
        target, target
    );

    debug!("Sending HTTP CONNECT request for {}", target);
    tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes()).await
        .map_err(|e| SyncthingError::connection(format!("proxy write failed: {}", e)))?;

    // 读取响应（简单解析，期待 200）
    let mut buf = vec![0u8; 1024];
    let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await
        .map_err(|e| SyncthingError::connection(format!("proxy read failed: {}", e)))?;

    let response = String::from_utf8_lossy(&buf[..n]);
    debug!("Proxy response: {}", response.lines().next().unwrap_or(""));

    if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
        return Err(SyncthingError::connection(format!(
            "proxy returned non-200: {}",
            response.lines().next().unwrap_or("unknown")
        )));
    }

    info!("HTTP CONNECT tunnel established to {}", target);
    Ok(stream)
}

/// 代理管道（底层就是 TcpStream，隧道已建立）
struct ProxyPipe {
    stream: TcpStream,
    local_addr: Option<SocketAddr>,
    peer_addr: Option<SocketAddr>,
}

impl AsyncRead for ProxyPipe {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for ProxyPipe {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

impl ReliablePipe for ProxyPipe {
    fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Proxy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_config_from_env_http() {
        // 注意：此测试会读取真实环境变量，在 CI 中可能不稳定
        // 仅验证解析逻辑
        let cfg = ProxyConfig::parse_http_url("127.0.0.1:8080");
        assert!(cfg.is_some());
        assert!(matches!(cfg.unwrap().proxy_type, ProxyType::HttpConnect));
    }

    #[test]
    fn test_proxy_config_from_env_socks() {
        let cfg = ProxyConfig::parse_socks_url("socks5://127.0.0.1:1080");
        assert!(cfg.is_some());
        assert!(matches!(cfg.unwrap().proxy_type, ProxyType::Socks5));
    }
}
