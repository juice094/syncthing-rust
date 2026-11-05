//! NAT port mapping client (Portmapper).
//!
//! (reference: Tailscale net/portmapper/ — portmapper.go, upnp.go, pmp.go, pcp.go)

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use syncthing_core::Result;
use tracing::{info, warn};

pub mod pcp;
pub mod pmp;
pub mod upnp;

/// Default port mapping lifetime in seconds (RFC-recommended 2 hours).
pub(crate) const PORT_MAP_LIFETIME_SEC: u32 = 7200;

/// Discovers the default gateway's IPv4 address via netdev.
async fn discover_gateway() -> Option<Ipv4Addr> {
    tokio::task::spawn_blocking(|| match netdev::get_default_gateway() {
        Ok(gw) => gw.ipv4.into_iter().next(),
        Err(_) => None,
    })
    .await
    .ok()
    .flatten()
}

/// Port mapping result.
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
    /// Returns the external address.
    pub fn external_addr(&self) -> SocketAddr {
        self.external
    }

    /// Returns when the mapping expires.
    pub fn good_until(&self) -> Instant {
        self.good_until
    }

    /// Returns the recommended renewal time.
    pub fn renew_after(&self) -> Instant {
        self.renew_after
    }

    /// Releases the mapping.
    ///
    /// # Errors
    /// Returns an error if the release packet cannot be sent.
    pub async fn release(&self) -> Result<()> {
        match &self.inner {
            MappingInner::Upnp(state) => upnp::release_mapping(state).await,
            MappingInner::Pmp(state) => release_pmp_mapping(state).await,
            MappingInner::Pcp(state) => release_pcp_mapping(state).await,
        }
    }
}

/// Releases a PMP mapping (sends a lifetime=0 UDP request).
async fn release_pmp_mapping(state: &pmp::PmpMappingState) -> Result<()> {
    let pkt = pmp::build_pmp_request_mapping_packet(state.internal_port, state.external_port, 0);
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| {
            syncthing_core::SyncthingError::connection(format!("PMP release bind failed: {}", e))
        })?;
    socket.send_to(&pkt, state.gateway).await.map_err(|e| {
        syncthing_core::SyncthingError::connection(format!("PMP release send failed: {}", e))
    })?;
    Ok(())
}

/// Releases a PCP mapping (sends a lifetime=0 UDP request).
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
        .map_err(|e| {
            syncthing_core::SyncthingError::connection(format!("PCP release bind failed: {}", e))
        })?;
    socket.send_to(&pkt, state.gateway).await.map_err(|e| {
        syncthing_core::SyncthingError::connection(format!("PCP release send failed: {}", e))
    })?;
    Ok(())
}

/// Port mapping client.
///
/// Tries to obtain a port mapping in PCP -> NAT-PMP -> UPnP order.
#[derive(Debug)]
pub struct PortMapper {
    local_addr: SocketAddr,
    /// Cache of the last successful mapping, for renewal or release.
    last_mapping: Option<Mapping>,
}

impl PortMapper {
    /// Creates a new port mapping client.
    pub fn new() -> Self {
        Self {
            local_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            last_mapping: None,
        }
    }

    /// Sets the local address.
    pub fn with_local_addr(mut self, addr: SocketAddr) -> Self {
        self.local_addr = addr;
        self
    }

    /// Allocates a port mapping, trying PCP -> NAT-PMP -> UPnP in order.
    ///
    /// # Errors
    /// Returns the UPnP error when all methods fail.
    pub async fn allocate_port(&mut self, local_port: u16) -> Result<Mapping> {
        let gateway = discover_gateway().await;
        let my_ip = self.local_ipv4().await;

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

        if let Some(gw) = gateway {
            let pmp_gw = SocketAddr::from((gw, pmp::PMP_DEFAULT_PORT));
            if pmp::probe_gateway(pmp_gw).await {
                match pmp::allocate_port(pmp_gw, local_port).await {
                    Ok((external, state)) => {
                        info!(
                            "NAT-PMP port mapping succeeded: {} -> {}",
                            local_port, external
                        );
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

        self.allocate_upnp(local_port).await
    }

    async fn allocate_upnp(&mut self, local_port: u16) -> Result<Mapping> {
        let (external, state) = upnp::allocate_port(self.local_addr, local_port).await?;
        info!(
            "UPnP port mapping succeeded: {} -> {}",
            local_port, external
        );
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

    /// Returns the local IPv4 address (used as the Client IP in PCP requests).
    async fn local_ipv4(&self) -> Option<Ipv4Addr> {
        if let std::net::IpAddr::V4(ip) = self.local_addr.ip() {
            if !ip.is_unspecified() {
                return Some(ip);
            }
        }
        tokio::task::spawn_blocking(|| match netdev::get_default_interface() {
            Ok(iface) => iface.ipv4_addrs().into_iter().next(),
            Err(_) => None,
        })
        .await
        .ok()
        .flatten()
    }

    /// Forces rediscovery by dropping the cached mapping information.
    pub fn invalidate(&mut self) {
        self.last_mapping = None;
    }
}

impl Default for PortMapper {
    fn default() -> Self {
        Self::new()
    }
}
