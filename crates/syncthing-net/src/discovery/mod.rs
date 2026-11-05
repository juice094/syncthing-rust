//! Device discovery manager.
//!
//! Integrates Local Discovery, STUN, and UPnP for device discovery.
//!
//! Example:
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
//!     // Get public addresses
//!     let public_addrs = manager.get_public_addresses().await;
//!     println!("Public addresses: {:?}", public_addrs);
//! }
//! ```

pub mod events;
pub mod global;
pub mod local;

use std::net::SocketAddr;
use tracing::{debug, info, warn};

use syncthing_core::Result;

use crate::stun::StunClient;
use crate::upnp::UpnpClient;

pub use events::{DiscoveryEvent, DiscoverySource};
pub use global::{GlobalDiscovery, ANNOUNCE_INTERVAL, DEFAULT_DISCOVERY_SERVER, RETRY_INTERVAL};
pub use local::{Announce, LocalDiscovery, LOCAL_DISCOVERY_MAGIC, LOCAL_DISCOVERY_PORT};

/// Discovery manager configuration.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Whether STUN is enabled.
    pub enable_stun: bool,
    /// Whether UPnP is enabled.
    pub enable_upnp: bool,
    /// STUN server list.
    pub stun_servers: Vec<String>,
    /// UPnP mapping duration in seconds.
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

/// Discovery manager.
///
/// Manages local address discovery, public address retrieval, and port mapping.
pub struct DiscoveryManager {
    /// Local listen address.
    local_addr: SocketAddr,
    /// Configuration.
    config: DiscoveryConfig,
    /// STUN client.
    stun_client: Option<StunClient>,
    /// UPnP client.
    upnp_client: Option<UpnpClient>,
}

impl DiscoveryManager {
    /// Creates a new discovery manager with default configuration.
    pub fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            config: DiscoveryConfig::default(),
            stun_client: None,
            upnp_client: None,
        }
    }

    /// Creates a discovery manager with explicit configuration.
    pub fn with_config(local_addr: SocketAddr, config: DiscoveryConfig) -> Self {
        let mut manager = Self::new(local_addr);
        manager.config = config;
        manager
    }

    /// Attaches a STUN client.
    pub fn with_stun(mut self, client: StunClient) -> Self {
        self.stun_client = Some(client);
        self
    }

    /// Attaches a UPnP client.
    pub fn with_upnp(mut self, client: UpnpClient) -> Self {
        self.upnp_client = Some(client);
        self
    }

    /// Initializes discovery services, configuring UPnP port mappings.
    pub async fn init(&mut self) -> Result<()> {
        if self.config.enable_upnp {
            if let Some(ref client) = self.upnp_client {
                let port = self.local_addr.port();
                match client
                    .add_tcp_mapping(port, self.config.upnp_duration)
                    .await
                {
                    Ok(_) => {
                        info!("UPnP TCP port mapping configured: {} -> {}", port, port);
                    }
                    Err(e) => {
                        warn!("Failed to configure UPnP port mapping: {}", e);
                    }
                }

                match client
                    .add_udp_mapping(port, self.config.upnp_duration)
                    .await
                {
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

    /// Returns the list of addresses usable for public discovery.
    pub async fn get_public_addresses(&self) -> Vec<String> {
        let mut addrs = vec![];

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

    /// Returns the local listen address.
    pub fn local_address(&self) -> SocketAddr {
        self.local_addr
    }

    /// Shuts down discovery services, cleaning up UPnP port mappings.
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

/// Address information.
#[derive(Debug, Clone)]
pub struct AddressInfo {
    /// Address string.
    pub address: String,
    /// Address type (local/stun/upnp).
    pub addr_type: AddressSource,
}

/// Address type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSource {
    /// Locally determined address.
    Local,
    /// Address learned via STUN.
    Stun,
    /// Address learned via UPnP.
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
