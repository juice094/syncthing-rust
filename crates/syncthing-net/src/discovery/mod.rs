//! 设备发现管理器
//!
//! 集成 Local Discovery、STUN 和 UPnP 支持设备发现。
//!
//! 使用示例:
//! ```rust,ignore
//! use syncthing_net::{DiscoveryManager, StunClient, UpnpClient};
//! use std::net::SocketAddr;
//!
//! async fn setup_discovery() {
//!     let local_addr = SocketAddr::from(([0, 0, 0, 0], 22000));
//!     
//!     let manager = DiscoveryManager::new(local_addr)
//!         .with_stun(StunClient::new())
//!         .with_upnp(UpnpClient::new(local_addr));
//!     
//!     // 获取公网地址
//!     let public_addrs = manager.get_public_addresses().await;
//!     println!("Public addresses: {:?}", public_addrs);
//! }
//! ```

pub mod events;
pub mod local;
pub mod global;

use std::net::SocketAddr;
use tracing::{debug, info, warn};

use syncthing_core::Result;

use crate::stun::StunClient;
use crate::upnp::UpnpClient;

pub use events::{DiscoveryEvent, DiscoverySource};
pub use local::{Announce, LocalDiscovery, LOCAL_DISCOVERY_MAGIC, LOCAL_DISCOVERY_PORT};
pub use global::{GlobalDiscovery, DEFAULT_DISCOVERY_SERVER, ANNOUNCE_INTERVAL, RETRY_INTERVAL};

/// 发现管理器配置
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// 启用 STUN
    pub enable_stun: bool,
    /// 启用 UPnP
    pub enable_upnp: bool,
    /// STUN 服务器列表
    pub stun_servers: Vec<String>,
    /// UPnP 映射持续时间（秒）
    pub upnp_duration: u32,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            enable_stun: true,
            enable_upnp: true,
            stun_servers: crate::stun::DEFAULT_STUN_SERVERS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            upnp_duration: crate::upnp::DEFAULT_MAPPING_DURATION,
        }
    }
}

/// 发现管理器
/// 
/// 管理本地地址发现、公网地址获取和端口映射
pub struct DiscoveryManager {
    /// 本地监听地址
    local_addr: SocketAddr,
    /// 配置
    config: DiscoveryConfig,
    /// STUN 客户端
    stun_client: Option<StunClient>,
    /// UPnP 客户端
    upnp_client: Option<UpnpClient>,
}

impl DiscoveryManager {
    /// 创建新的发现管理器
    pub fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            config: DiscoveryConfig::default(),
            stun_client: None,
            upnp_client: None,
        }
    }

    /// 使用配置创建
    pub fn with_config(local_addr: SocketAddr, config: DiscoveryConfig) -> Self {
        let mut manager = Self::new(local_addr);
        manager.config = config;
        manager
    }

    /// 设置 STUN 客户端
    pub fn with_stun(mut self, client: StunClient) -> Self {
        self.stun_client = Some(client);
        self
    }

    /// 设置 UPnP 客户端
    pub fn with_upnp(mut self, client: UpnpClient) -> Self {
        self.upnp_client = Some(client);
        self
    }

    /// 初始化并启动发现服务
    /// 
    /// 配置 UPnP 端口映射等
    pub async fn init(&mut self) -> Result<()> {
        // 尝试设置 UPnP 端口映射
        if self.config.enable_upnp {
            if let Some(ref client) = self.upnp_client {
                let port = self.local_addr.port();
                match client.add_tcp_mapping(port, self.config.upnp_duration).await {
                    Ok(_) => {
                        info!("UPnP TCP port mapping configured: {} -> {}", port, port);
                    }
                    Err(e) => {
                        warn!("Failed to configure UPnP port mapping: {}", e);
                    }
                }

                // 同时映射 UDP（用于未来扩展）
                match client.add_udp_mapping(port, self.config.upnp_duration).await {
                    Ok(_) => {
                        info!("UPnP UDP port mapping configured: {} -> {}", port, port);
                    }
                    Err(e) => {
                        debug!("Failed to configure UPnP UDP port mapping: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    /// 获取公网地址列表
    /// 
    /// 返回可用于公网发现的地址列表
    pub async fn get_public_addresses(&self) -> Vec<String> {
        let mut addrs = vec![];

        // STUN 获取公网地址
        if self.config.enable_stun {
            if let Some(ref stun) = self.stun_client {
                match stun.get_public_address().await {
                    Ok(addr) => {
                        info!("STUN returned public address: {}", addr);
                        addrs.push(format!("tcp://{}", addr));
                    }
                    Err(e) => {
                        warn!("STUN failed to get public address: {}", e);
                    }
                }
            }
        }

        // UPnP 外部地址
        if self.config.enable_upnp {
            if let Some(ref upnp) = self.upnp_client {
                match upnp.get_external_ip().await {
                    Ok(ip) => {
                        let port = self.local_addr.port();
                        let addr = SocketAddr::new(ip, port);
                        info!("UPnP external address: {}", addr);
                        addrs.push(format!("tcp://{}", addr));
                    }
                    Err(e) => {
                        debug!("UPnP failed to get external IP: {}", e);
                    }
                }
            }
        }

        addrs
    }

    /// 获取本地地址
    pub fn local_address(&self) -> SocketAddr {
        self.local_addr
    }

    /// 关闭发现服务
    /// 
    /// 清理 UPnP 端口映射等
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(ref client) = self.upnp_client {
            let port = self.local_addr.port();
            if let Err(e) = client.remove_tcp_mapping(port).await {
                warn!("Failed to remove UPnP TCP mapping: {}", e);
            }
            if let Err(e) = client.remove_udp_mapping(port).await {
                debug!("Failed to remove UPnP UDP mapping: {}", e);
            }
        }

        Ok(())
    }
}

/// 地址信息
#[derive(Debug, Clone)]
pub struct AddressInfo {
    /// 地址字符串
    pub address: String,
    /// 地址类型（local/stun/upnp）
    pub addr_type: AddressSource,
}

/// 地址类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSource {
    /// 本地地址
    Local,
    /// STUN 获取的地址
    Stun,
    /// UPnP 获取的地址
    Upnp,
}

impl std::fmt::Display for AddressSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressSource::Local => write!(f, "local"),
            AddressSource::Stun => write!(f, "stun"),
            AddressSource::Upnp => write!(f, "upnp"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovery_config_default() {
        let config = DiscoveryConfig::default();
        assert!(config.enable_stun);
        assert!(config.enable_upnp);
        assert!(!config.stun_servers.is_empty());
        assert_eq!(config.upnp_duration, crate::upnp::DEFAULT_MAPPING_DURATION);
    }

    #[test]
    fn test_discovery_manager_new() {
        let local_addr = SocketAddr::from(([127, 0, 0, 1], 22000));
        let manager = DiscoveryManager::new(local_addr);
        assert_eq!(manager.local_address(), local_addr);
    }

    #[test]
    fn test_address_type_display() {
        assert_eq!(AddressSource::Local.to_string(), "local");
        assert_eq!(AddressSource::Stun.to_string(), "stun");
        assert_eq!(AddressSource::Upnp.to_string(), "upnp");
    }
}
