//! UPnP/IGD client implementation.
//!
//! Used to automatically configure router port mappings
//! (reference: syncthing/lib/upnp/).

use std::net::SocketAddr;
use std::time::Duration;

use igd::aio::{search_gateway, Gateway};
use igd::{PortMappingProtocol, SearchOptions};
use tracing::{debug, error, info, warn};

use syncthing_core::{Result, SyncthingError};

/// Default port mapping duration in seconds (30 minutes).
///
/// UPnP mappings expire and must be renewed periodically.
pub const DEFAULT_MAPPING_DURATION: u32 = 1800;

/// Minimum renewal interval (25 minutes); renewals happen before expiry.
pub const MIN_RENEWAL_INTERVAL: Duration = Duration::from_secs(1500);

/// UPnP client.
#[derive(Debug, Clone)]
pub struct UpnpClient {
    /// Local address.
    local_addr: SocketAddr,
    /// Mapping description.
    description: String,
}

impl UpnpClient {
    /// Creates a new UPnP client.
    pub fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            description: "syncthing-rust".to_string(),
        }
    }

    /// Overrides the mapping description.
    pub fn with_description(mut self, description: String) -> Self {
        self.description = description;
        self
    }

    /// Searches the local network for a UPnP/IGD gateway.
    ///
    /// # Errors
    /// Returns an error if no gateway responds within the search timeout.
    pub async fn discover_gateway(&self) -> Result<Gateway> {
        debug!("Searching for UPnP/IGD gateway...");

        let options = SearchOptions {
            timeout: Some(Duration::from_secs(5)),
            ..Default::default()
        };

        let gateway = search_gateway(options).await.map_err(|e| {
            SyncthingError::connection(format!("failed to discover UPnP gateway: {}", e))
        })?;

        info!("Found UPnP gateway");
        Ok(gateway)
    }

    /// Adds a TCP port mapping.
    ///
    /// # Errors
    /// Returns an error if no gateway is found, IPv6 is used, or the gateway
    /// rejects the mapping. `duration` is in seconds; 0 means permanent.
    pub async fn add_tcp_mapping(&self, external_port: u16, duration: u32) -> Result<()> {
        self.add_port_mapping(
            external_port,
            self.local_addr.port(),
            PortMappingProtocol::TCP,
            duration,
        )
        .await
    }

    /// Adds a UDP port mapping.
    ///
    /// # Errors
    /// Returns an error if no gateway is found, IPv6 is used, or the gateway
    /// rejects the mapping. `duration` is in seconds; 0 means permanent.
    pub async fn add_udp_mapping(&self, external_port: u16, duration: u32) -> Result<()> {
        self.add_port_mapping(
            external_port,
            self.local_addr.port(),
            PortMappingProtocol::UDP,
            duration,
        )
        .await
    }

    /// Adds a port mapping (internal implementation).
    async fn add_port_mapping(
        &self,
        external_port: u16,
        internal_port: u16,
        protocol: PortMappingProtocol,
        duration: u32,
    ) -> Result<()> {
        let gateway = self.discover_gateway().await?;

        debug!(
            "Adding UPnP port mapping: {} {} -> {} (duration: {}s)",
            protocol, external_port, internal_port, duration
        );

        let local_v4 = match self.local_addr {
            SocketAddr::V4(v4) => v4,
            SocketAddr::V6(_) => {
                return Err(SyncthingError::config("IPv6 not supported for UPnP"));
            }
        };

        gateway
            .add_port(
                protocol,
                external_port,
                local_v4,
                duration,
                &self.description,
            )
            .await
            .map_err(|e| {
                error!("Failed to add UPnP port mapping: {}", e);
                SyncthingError::connection(format!("failed to add port mapping: {}", e))
            })?;

        info!(
            "Successfully added UPnP port mapping: {} {} -> {}",
            protocol, external_port, internal_port
        );

        Ok(())
    }

    /// Removes a port mapping. Missing gateway is treated as success.
    ///
    /// # Errors
    /// Returns an error if the gateway rejects the removal.
    pub async fn remove_port_mapping(
        &self,
        external_port: u16,
        protocol: PortMappingProtocol,
    ) -> Result<()> {
        let gateway = match self.discover_gateway().await {
            Ok(g) => g,
            Err(e) => {
                warn!("Gateway not found for removing mapping: {}", e);
                return Ok(());
            }
        };

        debug!("Removing UPnP port mapping: {} {}", protocol, external_port);

        gateway
            .remove_port(protocol, external_port)
            .await
            .map_err(|e| {
                warn!("Failed to remove UPnP port mapping: {}", e);
                SyncthingError::connection(format!("failed to remove port mapping: {}", e))
            })?;

        info!(
            "Successfully removed UPnP port mapping: {} {}",
            protocol, external_port
        );

        Ok(())
    }

    /// Removes a TCP port mapping.
    ///
    /// # Errors
    /// See `remove_port_mapping`.
    pub async fn remove_tcp_mapping(&self, external_port: u16) -> Result<()> {
        self.remove_port_mapping(external_port, PortMappingProtocol::TCP)
            .await
    }

    /// Removes a UDP port mapping.
    ///
    /// # Errors
    /// See `remove_port_mapping`.
    pub async fn remove_udp_mapping(&self, external_port: u16) -> Result<()> {
        self.remove_port_mapping(external_port, PortMappingProtocol::UDP)
            .await
    }

    /// Returns the router's external IP as reported by the UPnP gateway.
    ///
    /// # Errors
    /// Returns an error if no gateway is found or the query fails.
    pub async fn get_external_ip(&self) -> Result<std::net::IpAddr> {
        let gateway = self.discover_gateway().await?;

        let ip = gateway
            .get_external_ip()
            .await
            .map_err(|e| SyncthingError::connection(format!("failed to get external IP: {}", e)))?;

        info!("UPnP external IP: {}", ip);
        Ok(ip.into())
    }

    /// Checks whether a matching port mapping exists (false when no gateway).
    ///
    /// # Errors
    /// This function swallows gateway/query errors and reports them as `false`;
    /// the returned `Result` is reserved for future use.
    pub async fn has_mapping(
        &self,
        external_port: u16,
        protocol: PortMappingProtocol,
    ) -> Result<bool> {
        let Ok(gateway) = self.discover_gateway().await else {
            return Ok(false);
        };

        match gateway
            .get_generic_port_mapping_entry(external_port as u32)
            .await
        {
            Ok(entry) => Ok(entry.protocol == protocol),
            Err(_) => Ok(false),
        }
    }
}

/// Port mapping entry.
#[derive(Debug, Clone)]
pub struct PortMapping {
    /// External (router-side) port.
    pub external_port: u16,
    /// Internal (local) port.
    pub internal_port: u16,
    /// Protocol.
    pub protocol: PortMappingProtocol,
    /// Duration in seconds (0 = permanent).
    pub duration: u32,
    /// Creation time.
    pub created_at: std::time::Instant,
}

impl PortMapping {
    /// Returns whether the mapping is close enough to expiry to need renewal.
    pub fn is_expiring_soon(&self) -> bool {
        self.is_expiring_soon_at(std::time::Instant::now())
    }

    fn is_expiring_soon_at(&self, now: std::time::Instant) -> bool {
        if self.duration == 0 {
            return false;
        }

        let elapsed = if now > self.created_at {
            now.duration_since(self.created_at).as_secs() as u32
        } else {
            0
        };
        let remaining = self.duration.saturating_sub(elapsed);

        remaining < MIN_RENEWAL_INTERVAL.as_secs() as u32
    }
}

/// UPnP port mapping manager.
///
/// Manages multiple mappings and renews them automatically.
pub struct UpnpMappingManager {
    client: UpnpClient,
    mappings: std::sync::Arc<tokio::sync::RwLock<Vec<PortMapping>>>,
    renewal_interval: Duration,
}

impl UpnpMappingManager {
    /// Creates a new mapping manager with the default renewal interval.
    pub fn new(client: UpnpClient) -> Self {
        Self {
            client,
            mappings: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            renewal_interval: MIN_RENEWAL_INTERVAL,
        }
    }

    /// Overrides the renewal interval.
    pub fn with_renewal_interval(mut self, interval: Duration) -> Self {
        self.renewal_interval = interval;
        self
    }

    /// Adds a mapping via the gateway and starts managing it.
    ///
    /// # Errors
    /// Returns an error if the gateway rejects the mapping.
    pub async fn add_mapping(
        &self,
        external_port: u16,
        internal_port: u16,
        protocol: PortMappingProtocol,
        duration: u32,
    ) -> Result<()> {
        self.client
            .add_port_mapping(external_port, internal_port, protocol, duration)
            .await?;

        let mapping = PortMapping {
            external_port,
            internal_port,
            protocol,
            duration,
            created_at: std::time::Instant::now(),
        };

        let mut mappings = self.mappings.write().await;

        mappings.retain(|m| !(m.external_port == external_port && m.protocol == protocol));

        mappings.push(mapping);

        info!(
            "Added managed UPnP mapping: {} {} -> {} (duration: {}s)",
            protocol, external_port, internal_port, duration
        );

        Ok(())
    }

    /// Removes a mapping from the gateway and from management.
    ///
    /// # Errors
    /// Returns an error if the gateway rejects the removal.
    pub async fn remove_mapping(
        &self,
        external_port: u16,
        protocol: PortMappingProtocol,
    ) -> Result<()> {
        self.client
            .remove_port_mapping(external_port, protocol)
            .await?;

        let mut mappings = self.mappings.write().await;
        mappings.retain(|m| !(m.external_port == external_port && m.protocol == protocol));

        Ok(())
    }

    /// Runs the automatic renewal loop (never returns).
    pub async fn start_renewal_task(&self) -> Result<()> {
        let mut interval = tokio::time::interval(self.renewal_interval);
        let client = self.client.clone();
        let mappings = self.mappings.clone();

        loop {
            interval.tick().await;

            let mut mappings_guard = mappings.write().await;

            for mapping in mappings_guard.iter_mut() {
                if mapping.is_expiring_soon() {
                    info!(
                        "Renewing UPnP mapping: {} {} -> {}",
                        mapping.protocol, mapping.external_port, mapping.internal_port
                    );

                    match client
                        .add_port_mapping(
                            mapping.external_port,
                            mapping.internal_port,
                            mapping.protocol,
                            mapping.duration,
                        )
                        .await
                    {
                        Ok(_) => {
                            mapping.created_at = std::time::Instant::now();
                            info!("Successfully renewed UPnP mapping");
                        }
                        Err(e) => {
                            warn!("Failed to renew UPnP mapping: {}", e);
                        }
                    }
                }
            }
        }
    }

    /// Removes all managed mappings from the gateway and clears the list.
    ///
    /// # Errors
    /// Per-mapping failures are logged; this currently always returns `Ok`.
    pub async fn cleanup(&self) -> Result<()> {
        let mappings = self.mappings.read().await;

        for mapping in mappings.iter() {
            if let Err(e) = self
                .client
                .remove_port_mapping(mapping.external_port, mapping.protocol)
                .await
            {
                warn!(
                    "Failed to remove UPnP mapping {} {}: {}",
                    mapping.protocol, mapping.external_port, e
                );
            }
        }

        drop(mappings);
        self.mappings.write().await.clear();

        Ok(())
    }

    /// Returns the currently managed mappings.
    pub async fn list_mappings(&self) -> Vec<PortMapping> {
        self.mappings.read().await.clone()
    }
}

/// UPnP discovery result.
#[derive(Debug, Clone)]
pub struct UpnpDiscoveryResult {
    /// Whether UPnP is available.
    pub available: bool,
    /// External IP address.
    pub external_ip: Option<std::net::IpAddr>,
    /// Gateway information.
    pub gateway_info: Option<String>,
}

impl UpnpDiscoveryResult {
    /// Creates an "unavailable" result.
    pub fn unavailable() -> Self {
        Self {
            available: false,
            external_ip: None,
            gateway_info: None,
        }
    }
}

/// Runs UPnP discovery and returns the result, including the external IP.
pub async fn discover_upnp(local_addr: SocketAddr) -> UpnpDiscoveryResult {
    let client = UpnpClient::new(local_addr);

    match client.discover_gateway().await {
        Ok(_gateway) => match client.get_external_ip().await {
            Ok(external_ip) => UpnpDiscoveryResult {
                available: true,
                external_ip: Some(external_ip),
                gateway_info: Some("IGD Gateway found".to_string()),
            },
            Err(e) => {
                warn!("Failed to get external IP via UPnP: {}", e);
                UpnpDiscoveryResult::unavailable()
            }
        },
        Err(e) => {
            debug!("UPnP not available: {}", e);
            UpnpDiscoveryResult::unavailable()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upnp_client_new() {
        let local_addr = SocketAddr::from(([127, 0, 0, 1], 22000));
        let client = UpnpClient::new(local_addr);
        assert_eq!(client.local_addr, local_addr);
    }

    #[test]
    fn test_port_mapping_expiration() {
        // 创建一个长期的映射（1小时），新创建时不应该即将过期
        let mapping = PortMapping {
            external_port: 22000,
            internal_port: 22000,
            protocol: PortMappingProtocol::TCP,
            duration: 3600, // 1小时
            created_at: std::time::Instant::now(),
        };

        // 新创建的长期映射不应该即将过期
        assert!(!mapping.is_expiring_soon());

        // 创建一个即将过期的映射（已过去大部分时间）
        let now = std::time::Instant::now();
        let old_mapping = PortMapping {
            external_port: 22000,
            internal_port: 22000,
            protocol: PortMappingProtocol::TCP,
            duration: 3600,  // 1小时
            created_at: now, // 用模拟的 "now" 来测试
        };

        // 应该即将过期（剩余2分钟，小于 MIN_RENEWAL_INTERVAL 的25分钟）
        assert!(old_mapping.is_expiring_soon_at(now + Duration::from_secs(3500)));

        // 测试短持续时间映射（duration 小于 MIN_RENEWAL_INTERVAL）
        let short_mapping = PortMapping {
            external_port: 22000,
            internal_port: 22000,
            protocol: PortMappingProtocol::TCP,
            duration: 60, // 1分钟，小于 MIN_RENEWAL_INTERVAL
            created_at: std::time::Instant::now(),
        };

        // 短持续时间映射应该被认为即将过期（因为需要提前续约）
        assert!(short_mapping.is_expiring_soon());
    }

    #[test]
    fn test_upnp_discovery_result() {
        let result = UpnpDiscoveryResult::unavailable();
        assert!(!result.available);
        assert!(result.external_ip.is_none());
        assert!(result.gateway_info.is_none());
    }
}
