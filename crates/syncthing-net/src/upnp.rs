//! UPnP/IGD 客户端实现
//!
//! 用于自动配置路由器端口映射
//! 参考: syncthing/lib/upnp/

use std::net::SocketAddr;
use std::time::Duration;

use igd::aio::{search_gateway, Gateway};
use igd::{PortMappingProtocol, SearchOptions};
use tracing::{debug, error, info, warn};

use syncthing_core::{SyncthingError, Result};

/// 默认端口映射持续时间（秒）
/// 
/// UPnP 映射有过期时间，需要定期续约
/// 默认 30 分钟
pub const DEFAULT_MAPPING_DURATION: u32 = 1800;

/// 最小续约间隔（秒）
/// 
/// 在映射过期前进行续约
pub const MIN_RENEWAL_INTERVAL: Duration = Duration::from_secs(1500); // 25分钟

/// UPnP 客户端
#[derive(Debug, Clone)]
pub struct UpnpClient {
    /// 本地地址
    local_addr: SocketAddr,
    /// 映射描述
    description: String,
}

impl UpnpClient {
    /// 创建新的 UPnP 客户端
    pub fn new(local_addr: SocketAddr) -> Self {
        Self {
            local_addr,
            description: "syncthing-rust".to_string(),
        }
    }

    /// 设置映射描述
    pub fn with_description(mut self, description: String) -> Self {
        self.description = description;
        self
    }

    /// 搜索并获取网关
    /// 
    /// 搜索本地网络中的 UPnP/IGD 网关
    pub async fn discover_gateway(&self) -> Result<Gateway> {
        debug!("Searching for UPnP/IGD gateway...");

        let options = SearchOptions {
            timeout: Some(Duration::from_secs(5)),
            ..Default::default()
        };

        let gateway = search_gateway(options).await
            .map_err(|e| SyncthingError::connection(format!("failed to discover UPnP gateway: {}", e)))?;

        info!("Found UPnP gateway");
        Ok(gateway)
    }

    /// 添加 TCP 端口映射
    /// 
    /// # 参数
    /// - `external_port`: 外部端口（路由器上的端口）
    /// - `duration`: 映射持续时间（秒），0 表示永久
    pub async fn add_tcp_mapping(&self, external_port: u16, duration: u32) -> Result<()> {
        self.add_port_mapping(external_port, self.local_addr.port(), PortMappingProtocol::TCP, duration)
            .await
    }

    /// 添加 UDP 端口映射
    /// 
    /// # 参数
    /// - `external_port`: 外部端口（路由器上的端口）
    /// - `duration`: 映射持续时间（秒），0 表示永久
    pub async fn add_udp_mapping(&self, external_port: u16, duration: u32) -> Result<()> {
        self.add_port_mapping(external_port, self.local_addr.port(), PortMappingProtocol::UDP, duration)
            .await
    }

    /// 添加端口映射（内部实现）
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

        // 转换为 IPv4 地址
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

    /// 移除端口映射
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

        info!("Successfully removed UPnP port mapping: {} {}", protocol, external_port);

        Ok(())
    }

    /// 移除 TCP 端口映射
    pub async fn remove_tcp_mapping(&self, external_port: u16) -> Result<()> {
        self.remove_port_mapping(external_port, PortMappingProtocol::TCP).await
    }

    /// 移除 UDP 端口映射
    pub async fn remove_udp_mapping(&self, external_port: u16) -> Result<()> {
        self.remove_port_mapping(external_port, PortMappingProtocol::UDP).await
    }

    /// 获取外部 IP 地址
    /// 
    /// 通过 UPnP 网关获取路由器的外部 IP
    pub async fn get_external_ip(&self) -> Result<std::net::IpAddr> {
        let gateway = self.discover_gateway().await?;

        let ip = gateway
            .get_external_ip()
            .await
            .map_err(|e| SyncthingError::connection(format!("failed to get external IP: {}", e)))?;

        info!("UPnP external IP: {}", ip);
        Ok(ip.into())
    }

    /// 检查端口映射是否存在
    pub async fn has_mapping(&self, external_port: u16, protocol: PortMappingProtocol) -> Result<bool> {
        let gateway = match self.discover_gateway().await {
            Ok(g) => g,
            Err(_) => return Ok(false),
        };

        match gateway.get_generic_port_mapping_entry(external_port as u32).await {
            Ok(entry) => {
                // 检查协议是否匹配
                Ok(entry.protocol == protocol)
            }
            Err(_) => Ok(false),
        }
    }
}

/// 端口映射条目
#[derive(Debug, Clone)]
pub struct PortMapping {
    /// 外部端口
    pub external_port: u16,
    /// 内部端口
    pub internal_port: u16,
    /// 协议
    pub protocol: PortMappingProtocol,
    /// 持续时间（秒）
    pub duration: u32,
    /// 创建时间
    pub created_at: std::time::Instant,
}

impl PortMapping {
    /// 检查映射是否即将过期
    pub fn is_expiring_soon(&self) -> bool {
        if self.duration == 0 {
            return false; // 永久映射
        }
        
        let elapsed = self.created_at.elapsed().as_secs() as u32;
        let remaining = self.duration.saturating_sub(elapsed);
        
        // 如果剩余时间少于最小续约间隔，认为即将过期
        remaining < MIN_RENEWAL_INTERVAL.as_secs() as u32
    }
}

/// UPnP 端口映射管理器
/// 
/// 管理多个端口映射，自动续约
pub struct UpnpMappingManager {
    client: UpnpClient,
    mappings: std::sync::Arc<tokio::sync::RwLock<Vec<PortMapping>>>,
    renewal_interval: Duration,
}

impl UpnpMappingManager {
    /// 创建新的映射管理器
    pub fn new(client: UpnpClient) -> Self {
        Self {
            client,
            mappings: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            renewal_interval: MIN_RENEWAL_INTERVAL,
        }
    }

    /// 设置续约间隔
    pub fn with_renewal_interval(mut self, interval: Duration) -> Self {
        self.renewal_interval = interval;
        self
    }

    /// 添加并管理一个映射
    pub async fn add_mapping(
        &self,
        external_port: u16,
        internal_port: u16,
        protocol: PortMappingProtocol,
        duration: u32,
    ) -> Result<()> {
        // 先添加映射
        self.client
            .add_port_mapping(external_port, internal_port, protocol, duration)
            .await?;

        // 记录映射
        let mapping = PortMapping {
            external_port,
            internal_port,
            protocol,
            duration,
            created_at: std::time::Instant::now(),
        };

        let mut mappings = self.mappings.write().await;
        
        // 移除已存在的相同映射
        mappings.retain(|m| {
            !(m.external_port == external_port && m.protocol == protocol)
        });
        
        mappings.push(mapping);

        info!(
            "Added managed UPnP mapping: {} {} -> {} (duration: {}s)",
            protocol, external_port, internal_port, duration
        );

        Ok(())
    }

    /// 移除映射
    pub async fn remove_mapping(&self, external_port: u16, protocol: PortMappingProtocol) -> Result<()> {
        self.client.remove_port_mapping(external_port, protocol).await?;

        let mut mappings = self.mappings.write().await;
        mappings.retain(|m| !(m.external_port == external_port && m.protocol == protocol));

        Ok(())
    }

    /// 启动自动续约任务
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

    /// 清理所有映射
    pub async fn cleanup(&self) -> Result<()> {
        let mappings = self.mappings.read().await;
        
        for mapping in mappings.iter() {
            if let Err(e) = self.client.remove_port_mapping(mapping.external_port, mapping.protocol).await {
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

    /// 获取当前管理的映射列表
    pub async fn list_mappings(&self) -> Vec<PortMapping> {
        self.mappings.read().await.clone()
    }
}

/// UPnP 发现结果
#[derive(Debug, Clone)]
pub struct UpnpDiscoveryResult {
    /// 是否支持 UPnP
    pub available: bool,
    /// 外部 IP 地址
    pub external_ip: Option<std::net::IpAddr>,
    /// 网关信息
    pub gateway_info: Option<String>,
}

impl UpnpDiscoveryResult {
    /// 创建不可用结果
    pub fn unavailable() -> Self {
        Self {
            available: false,
            external_ip: None,
            gateway_info: None,
        }
    }
}

/// 执行 UPnP 发现
/// 
/// 返回发现结果，包括外部 IP 等信息
pub async fn discover_upnp(local_addr: SocketAddr) -> UpnpDiscoveryResult {
    let client = UpnpClient::new(local_addr);

    match client.discover_gateway().await {
        Ok(_gateway) => {
            match client.get_external_ip().await {
                Ok(external_ip) => UpnpDiscoveryResult {
                    available: true,
                    external_ip: Some(external_ip),
                    gateway_info: Some("IGD Gateway found".to_string()),
                },
                Err(e) => {
                    warn!("Failed to get external IP via UPnP: {}", e);
                    UpnpDiscoveryResult::unavailable()
                }
            }
        }
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
        let old_mapping = PortMapping {
            external_port: 22000,
            internal_port: 22000,
            protocol: PortMappingProtocol::TCP,
            duration: 3600, // 1小时
            created_at: std::time::Instant::now() - Duration::from_secs(3500), // 已过去58分钟
        };

        // 应该即将过期（剩余2分钟，小于 MIN_RENEWAL_INTERVAL 的25分钟）
        assert!(old_mapping.is_expiring_soon());

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
