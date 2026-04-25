//! NAT 端口映射客户端 (Portmapper)
//!
//! 参考来源: Tailscale net/portmapper/
//! - portmapper.go
//! - upnp.go
//! - pmp.go
//! - pcp.go

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use syncthing_core::Result;
use tracing::{info, warn};

pub mod pcp;
pub mod pmp;
pub mod upnp;

/// 默认端口映射持续时间（秒）
/// RFC 推荐 2 小时
pub(crate) const PORT_MAP_LIFETIME_SEC: u32 = 7200;

/// 服务探测超时
#[allow(dead_code)]
pub(crate) const PORT_MAP_SERVICE_TIMEOUT: Duration = Duration::from_millis(250);

/// 通过 netdev 发现默认网关的 IPv4 地址
async fn discover_gateway() -> Option<Ipv4Addr> {
    tokio::task::spawn_blocking(|| {
        match netdev::get_default_gateway() {
            Ok(gw) => gw.ipv4.into_iter().next(),
            Err(_) => None,
        }
    })
    .await
    .ok()
    .flatten()
}

/// 端口映射结果
#[derive(Debug, Clone)]
pub struct Mapping {
    external: SocketAddr,
    good_until: Instant,
    renew_after: Instant,
    inner: MappingInner,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum MappingInner {
    Upnp(upnp::UpnpMappingState),
    Pmp(pmp::PmpMappingState),
    Pcp(pcp::PcpMappingState),
}

impl Mapping {
    /// 外部地址
    pub fn external_addr(&self) -> SocketAddr {
        self.external
    }

    /// 映射有效期
    pub fn good_until(&self) -> Instant {
        self.good_until
    }

    /// 建议续约时间
    pub fn renew_after(&self) -> Instant {
        self.renew_after
    }

    /// 释放映射
    pub async fn release(&self) -> Result<()> {
        match &self.inner {
            MappingInner::Upnp(state) => upnp::release_mapping(state).await,
            MappingInner::Pmp(state) => {
                release_pmp_mapping(state).await
            }
            MappingInner::Pcp(state) => {
                release_pcp_mapping(state).await
            }
        }
    }
}

/// 释放 PMP 映射（发送 lifetime=0 的 UDP 请求）
async fn release_pmp_mapping(state: &pmp::PmpMappingState) -> Result<()> {
    let pkt = pmp::build_pmp_request_mapping_packet(state.internal_port, state.external_port, 0);
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| syncthing_core::SyncthingError::connection(format!("PMP release bind failed: {}", e)))?;
    socket
        .send_to(&pkt, state.gateway)
        .await
        .map_err(|e| syncthing_core::SyncthingError::connection(format!("PMP release send failed: {}", e)))?;
    Ok(())
}

/// 释放 PCP 映射（发送 lifetime=0 的 UDP 请求）
async fn release_pcp_mapping(state: &pcp::PcpMappingState) -> Result<()> {
    let pkt = pcp::build_pcp_request_mapping_packet(
        state.my_ip,
        state.internal_port,
        state.external_port,
        0,
        std::net::Ipv4Addr::UNSPECIFIED,
    );
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| syncthing_core::SyncthingError::connection(format!("PCP release bind failed: {}", e)))?;
    socket
        .send_to(&pkt, state.gateway)
        .await
        .map_err(|e| syncthing_core::SyncthingError::connection(format!("PCP release send failed: {}", e)))?;
    Ok(())
}

/// 端口映射客户端
///
/// 按 PCP -> NAT-PMP -> UPnP 的顺序尝试获取端口映射
#[derive(Debug)]
pub struct PortMapper {
    local_addr: SocketAddr,
    /// 上次成功获取的映射缓存，用于续约或释放
    last_mapping: Option<Mapping>,
}

impl PortMapper {
    /// 创建新的端口映射客户端
    pub fn new() -> Self {
        Self {
            local_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            last_mapping: None,
        }
    }

    /// 设置本地地址
    pub fn with_local_addr(mut self, addr: SocketAddr) -> Self {
        self.local_addr = addr;
        self
    }

    /// 分配端口映射
    ///
    /// 按 PCP -> NAT-PMP -> UPnP 的顺序尝试获取端口映射。
    pub async fn allocate_port(&mut self, local_port: u16) -> Result<Mapping> {
        let gateway = discover_gateway().await;
        let my_ip = self.local_ipv4().await;

        // 1. 尝试 PCP
        if let (Some(gw), Some(ip)) = (gateway, my_ip) {
            let pcp_gw = SocketAddr::from((gw, pcp::PCP_DEFAULT_PORT));
            if pcp::probe_gateway(pcp_gw, ip).await {
                match pcp::allocate_port(pcp_gw, local_port, ip).await {
                    Ok((external, state)) => {
                        info!("PCP port mapping succeeded: {} -> {}", local_port, external);
                        let lifetime = Duration::from_secs(pcp::PCP_MAP_LIFETIME_SEC as u64);
                        let now = Instant::now();
                        let mapping = Mapping {
                            external,
                            good_until: now + lifetime,
                            renew_after: now + lifetime / 2,
                            inner: MappingInner::Pcp(state),
                        };
                        self.last_mapping = Some(mapping.clone());
                        return Ok(mapping);
                    }
                    Err(e) => warn!("PCP allocation failed: {}", e),
                }
            }
        }

        // 2. 尝试 NAT-PMP
        if let Some(gw) = gateway {
            let pmp_gw = SocketAddr::from((gw, pmp::PMP_DEFAULT_PORT));
            if pmp::probe_gateway(pmp_gw).await {
                match pmp::allocate_port(pmp_gw, local_port).await {
                    Ok((external, state)) => {
                        info!("NAT-PMP port mapping succeeded: {} -> {}", local_port, external);
                        let lifetime = Duration::from_secs(pmp::PMP_MAP_LIFETIME_SEC as u64);
                        let now = Instant::now();
                        let mapping = Mapping {
                            external,
                            good_until: now + lifetime,
                            renew_after: now + lifetime / 2,
                            inner: MappingInner::Pmp(state),
                        };
                        self.last_mapping = Some(mapping.clone());
                        return Ok(mapping);
                    }
                    Err(e) => warn!("NAT-PMP allocation failed: {}", e),
                }
            }
        }

        // 3. 回退到 UPnP
        self.allocate_upnp(local_port).await
    }

    async fn allocate_upnp(&mut self, local_port: u16) -> Result<Mapping> {
        let (external, state) = upnp::allocate_port(self.local_addr, local_port).await?;
        info!("UPnP port mapping succeeded: {} -> {}", local_port, external);
        let now = Instant::now();
        let lifetime = Duration::from_secs(PORT_MAP_LIFETIME_SEC as u64);
        let mapping = Mapping {
            external,
            good_until: now + lifetime,
            renew_after: now + lifetime / 2,
            inner: MappingInner::Upnp(state),
        };
        self.last_mapping = Some(mapping.clone());
        Ok(mapping)
    }

    /// 获取本地 IPv4 地址（用于 PCP 请求中的 Client IP）
    async fn local_ipv4(&self) -> Option<Ipv4Addr> {
        // 如果 local_addr 是具体的 IPv4，直接使用
        if let std::net::IpAddr::V4(ip) = self.local_addr.ip() {
            if !ip.is_unspecified() {
                return Some(ip);
            }
        }
        // 否则通过 netdev 获取默认接口的 IPv4
        tokio::task::spawn_blocking(|| {
            match netdev::get_default_interface() {
                Ok(iface) => iface.ipv4_addrs().into_iter().next(),
                Err(_) => None,
            }
        })
        .await
        .ok()
        .flatten()
    }

    /// 强制重新发现：清除缓存的映射信息
    pub fn invalidate(&mut self) {
        self.last_mapping = None;
    }
}

impl Default for PortMapper {
    fn default() -> Self {
        Self::new()
    }
}
