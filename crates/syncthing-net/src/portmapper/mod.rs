//! NAT 端口映射客户端 (Portmapper)
//!
//! 参考来源: Tailscale net/portmapper/
//! - portmapper.go
//! - upnp.go
//! - pmp.go
//! - pcp.go

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use syncthing_core::Result;

pub mod pcp;
pub mod pmp;
pub mod upnp;

/// 默认端口映射持续时间（秒）
/// RFC 推荐 2 小时
pub(crate) const PORT_MAP_LIFETIME_SEC: u32 = 7200;

/// 服务探测超时
#[allow(dead_code)]
pub(crate) const PORT_MAP_SERVICE_TIMEOUT: Duration = Duration::from_millis(250);

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
    Pcp,
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
            MappingInner::Pmp(_state) => {
                // TODO: 实现 PMP 映射释放
                Ok(())
            }
            MappingInner::Pcp => {
                // TODO: 实现 PCP 映射释放
                Ok(())
            }
        }
    }
}

/// 端口映射客户端
///
/// 按 PCP -> NAT-PMP -> UPnP 的顺序尝试获取端口映射
#[derive(Debug)]
pub struct PortMapper {
    local_addr: SocketAddr,
}

impl PortMapper {
    /// 创建新的端口映射客户端
    pub fn new() -> Self {
        Self {
            local_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
        }
    }

    /// 设置本地地址
    pub fn with_local_addr(mut self, addr: SocketAddr) -> Self {
        self.local_addr = addr;
        self
    }

    /// 分配端口映射
    ///
    /// 当前实现直接尝试 UPnP，PCP 和 NAT-PMP 留待后续实现。
    pub async fn allocate_port(&self, local_port: u16) -> Result<Mapping> {
        self.allocate_upnp(local_port).await
    }

    async fn allocate_upnp(&self, local_port: u16) -> Result<Mapping> {
        let (external, state) = upnp::allocate_port(self.local_addr, local_port).await?;
        let now = Instant::now();
        let lifetime = Duration::from_secs(PORT_MAP_LIFETIME_SEC as u64);
        Ok(Mapping {
            external,
            good_until: now + lifetime,
            renew_after: now + lifetime / 2,
            inner: MappingInner::Upnp(state),
        })
    }

    /// 强制重新发现
    pub fn invalidate(&mut self) {
        // TODO: 清除缓存的服务发现信息
    }
}

impl Default for PortMapper {
    fn default() -> Self {
        Self::new()
    }
}
