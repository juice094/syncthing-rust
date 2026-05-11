//! STUN 客户端实现
//!
//! 用于获取公网 IP:Port，支持 NAT 穿透。
//! 部分逻辑参考了 Tailscale 的 STUN 实现：
//! <https://github.com/tailscale/tailscale/blob/main/net/stun/stun.go>

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use rand::random;
use tokio::net::UdpSocket;
use tokio::time::timeout;
use tracing::{info, warn};

use syncthing_core::{Result, SyncthingError};

const ATTR_SOFTWARE: u16 = 0x8022;
const ATTR_FINGERPRINT: u16 = 0x8028;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const ATTR_XOR_MAPPED_ADDRESS_ALT: u16 = 0x8020;

const BINDING_REQUEST: [u8; 2] = [0x00, 0x01];
const BINDING_SUCCESS_RESPONSE: [u8; 2] = [0x01, 0x01];
const MAGIC_COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];
const HEADER_LEN: usize = 20;
const LEN_FINGERPRINT: usize = 8;

/// 默认 STUN 服务器列表
pub const DEFAULT_STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun.syncthing.net:3478",
    "stun1.l.google.com:19302",
    "stun2.l.google.com:19302",
    "stun3.l.google.com:19302",
    "stun4.l.google.com:19302",
];

/// STUN 请求超时时间
const STUN_TIMEOUT: Duration = Duration::from_secs(5);

const SOFTWARE: &str = "syncthing-rust";

/// Transaction ID (12 bytes)
pub type TxId = [u8; 12];

/// 生成随机 Transaction ID
pub fn new_tx_id() -> TxId {
    random()
}

/// 构建 STUN Binding Request（包含 SOFTWARE 与可选的 FINGERPRINT）
pub fn build_binding_request(tx_id: TxId) -> Vec<u8> {
    let len_software = 4 + SOFTWARE.len();
    let mut b = Vec::with_capacity(HEADER_LEN + len_software + LEN_FINGERPRINT);

    // Header
    b.extend_from_slice(&BINDING_REQUEST);
    b.extend_from_slice(&((len_software + LEN_FINGERPRINT) as u16).to_be_bytes());
    b.extend_from_slice(&MAGIC_COOKIE);
    b.extend_from_slice(&tx_id);

    // SOFTWARE attribute
    b.extend_from_slice(&ATTR_SOFTWARE.to_be_bytes());
    b.extend_from_slice(&(SOFTWARE.len() as u16).to_be_bytes());
    b.extend_from_slice(SOFTWARE.as_bytes());

    // FINGERPRINT attribute
    let fp = fingerprint(&b);
    b.extend_from_slice(&ATTR_FINGERPRINT.to_be_bytes());
    b.extend_from_slice(&4u16.to_be_bytes());
    b.extend_from_slice(&fp.to_be_bytes());

    b
}

/// CRC-32/IEEE 查表（用于 FINGERPRINT）
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i: u32 = 0;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = 0xedb88320 ^ (crc >> 1);
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc = CRC32_TABLE[((crc ^ (byte as u32)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

fn fingerprint(data: &[u8]) -> u32 {
    crc32_ieee(data) ^ 0x5354554e
}

/// 判断数据包是否为 STUN 消息
pub fn is_stun_packet(data: &[u8]) -> bool {
    data.len() >= HEADER_LEN && data[0] & 0b11000000 == 0 && data[4..8] == MAGIC_COOKIE
}

/// 解析 STUN Binding Success Response，返回 Transaction ID 与映射地址。
/// 优先读取 XOR-MAPPED-ADDRESS，不存在时回退到 MAPPED-ADDRESS。
pub fn parse_response(data: &[u8]) -> Result<(TxId, SocketAddr)> {
    if !is_stun_packet(data) {
        return Err(SyncthingError::protocol("response is not a STUN packet"));
    }

    let mut tx_id = TxId::default();
    tx_id.copy_from_slice(&data[8..20]);

    if data[0..2] != BINDING_SUCCESS_RESPONSE {
        return Err(SyncthingError::protocol(
            "STUN packet is not a success response",
        ));
    }

    let attrs_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let mut attrs = &data[HEADER_LEN..];
    if attrs_len > attrs.len() {
        return Err(SyncthingError::protocol(
            "STUN response has malformed attributes",
        ));
    }
    attrs = &attrs[..attrs_len];

    let mut addr: Option<SocketAddr> = None;
    let mut fallback_addr: Option<SocketAddr> = None;

    while !attrs.is_empty() {
        if attrs.len() < 4 {
            return Err(SyncthingError::protocol(
                "STUN response has malformed attributes",
            ));
        }
        let attr_type = u16::from_be_bytes([attrs[0], attrs[1]]);
        let attr_len = u16::from_be_bytes([attrs[2], attrs[3]]) as usize;
        let attr_len_padded = (attr_len + 3) & !3;
        attrs = &attrs[4..];
        if attr_len_padded > attrs.len() {
            return Err(SyncthingError::protocol(
                "STUN response has malformed attributes",
            ));
        }
        let attr_value = &attrs[..attr_len];
        attrs = &attrs[attr_len_padded..];

        match attr_type {
            ATTR_XOR_MAPPED_ADDRESS | ATTR_XOR_MAPPED_ADDRESS_ALT => {
                if let Ok((ip, port)) = parse_xor_mapped_address(tx_id, attr_value) {
                    addr = Some(SocketAddr::new(ip, port));
                }
            }
            ATTR_MAPPED_ADDRESS => {
                if let Ok((ip, port)) = parse_mapped_address(attr_value) {
                    fallback_addr = Some(SocketAddr::new(ip, port));
                }
            }
            _ => {}
        }
    }

    if let Some(addr) = addr {
        Ok((tx_id, addr))
    } else if let Some(addr) = fallback_addr {
        Ok((tx_id, addr))
    } else {
        Err(SyncthingError::protocol(
            "STUN response missing mapped address",
        ))
    }
}

fn parse_xor_mapped_address(tx_id: TxId, b: &[u8]) -> Result<(IpAddr, u16)> {
    if b.len() < 4 {
        return Err(SyncthingError::protocol(
            "malformed XOR-MAPPED-ADDRESS attribute",
        ));
    }
    let xor_port = u16::from_be_bytes([b[2], b[3]]);
    let port = xor_port ^ 0x2112;
    let addr_field = &b[4..];
    let addr_len = family_addr_len(b[1]);
    if addr_len == 0 || addr_field.len() < addr_len {
        return Err(SyncthingError::protocol(
            "malformed XOR-MAPPED-ADDRESS attribute",
        ));
    }
    let xor_addr = &addr_field[..addr_len];
    let mut addr = vec![0u8; addr_len];
    for i in 0..addr_len {
        if i < MAGIC_COOKIE.len() {
            addr[i] = xor_addr[i] ^ MAGIC_COOKIE[i];
        } else {
            addr[i] = xor_addr[i] ^ tx_id[i - MAGIC_COOKIE.len()];
        }
    }
    let ip = ip_from_bytes(&addr)?;
    Ok((ip, port))
}

fn parse_mapped_address(b: &[u8]) -> Result<(IpAddr, u16)> {
    if b.len() < 4 {
        return Err(SyncthingError::protocol(
            "malformed MAPPED-ADDRESS attribute",
        ));
    }
    let port = ((b[2] as u16) << 8) | (b[3] as u16);
    let addr_field = &b[4..];
    let addr_len = family_addr_len(b[1]);
    if addr_len == 0 || addr_field.len() < addr_len {
        return Err(SyncthingError::protocol(
            "malformed MAPPED-ADDRESS attribute",
        ));
    }
    let ip = ip_from_bytes(&addr_field[..addr_len])?;
    Ok((ip, port))
}

fn family_addr_len(fam: u8) -> usize {
    match fam {
        0x01 => 4,  // IPv4
        0x02 => 16, // IPv6
        _ => 0,
    }
}

fn ip_from_bytes(b: &[u8]) -> Result<IpAddr> {
    match b.len() {
        4 => {
            let octets = [b[0], b[1], b[2], b[3]];
            Ok(IpAddr::V4(Ipv4Addr::from(octets)))
        }
        16 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(b);
            Ok(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => Err(SyncthingError::protocol("invalid IP address length")),
    }
}

/// NAT 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatType {
    /// 无 NAT 或 Full Cone — 映射稳定，任何外部地址可直接连接
    Open,
    /// Restricted / Port Restricted Cone — 映射稳定，但需先向外发送包
    Restricted,
    /// Symmetric NAT — 映射随目标地址变化，P2P 困难
    Symmetric,
    /// UDP 完全不通
    Blocked,
    /// 无法判断（服务器不足或查询失败）
    Unknown,
}

impl NatType {
    /// 该 NAT 类型下 P2P 直连是否可行
    pub fn is_p2p_feasible(self) -> bool {
        matches!(self, NatType::Open | NatType::Restricted)
    }

    /// 是否应直接 fallback 到 Relay
    pub fn needs_relay(self) -> bool {
        matches!(
            self,
            NatType::Symmetric | NatType::Blocked | NatType::Unknown
        )
    }
}

/// 使用已有 UDP socket 向指定 STUN 服务器查询
async fn query_with_socket(
    socket: &UdpSocket,
    stun_server: &str,
    timeout_duration: Duration,
) -> Result<SocketAddr> {
    let server_addr = tokio::net::lookup_host(stun_server)
        .await
        .map_err(|e| {
            SyncthingError::config(format!(
                "failed to resolve STUN server '{}': {}",
                stun_server, e
            ))
        })?
        .next()
        .ok_or_else(|| {
            SyncthingError::config(format!(
                "STUN server '{}' resolved to no addresses",
                stun_server
            ))
        })?;

    let tx_id = new_tx_id();
    let request = build_binding_request(tx_id);

    let response = timeout(timeout_duration, async {
        socket.send_to(&request, server_addr).await?;
        let mut buf = [0u8; 1024];
        let (len, _) = socket.recv_from(&mut buf).await?;
        Ok::<Vec<u8>, std::io::Error>(buf[..len].to_vec())
    })
    .await
    .map_err(|_| SyncthingError::timeout("STUN request timeout"))?
    .map_err(|e| SyncthingError::connection(format!("STUN request failed: {}", e)))?;

    let (resp_tx_id, addr) = parse_response(&response)?;
    if resp_tx_id != tx_id {
        return Err(SyncthingError::protocol("transaction ID mismatch"));
    }
    Ok(addr)
}

/// 向指定 STUN 服务器发送 Binding Request 并返回公网地址
pub async fn query(stun_server: &str, timeout_duration: Duration) -> Result<SocketAddr> {
    let bind_addr = SocketAddr::from(([0, 0, 0, 0], 0));
    let socket = UdpSocket::bind(bind_addr)
        .await
        .map_err(|e| SyncthingError::connection(format!("failed to bind UDP socket: {}", e)))?;
    query_with_socket(&socket, stun_server, timeout_duration).await
}

/// STUN 客户端
#[derive(Debug, Clone)]
pub struct StunClient {
    /// STUN 服务器地址列表
    servers: Vec<String>,
    /// 本地绑定端口（0 表示随机）
    local_port: u16,
    /// 请求超时时间
    timeout: Duration,
}

impl Default for StunClient {
    fn default() -> Self {
        Self {
            servers: DEFAULT_STUN_SERVERS.iter().map(|s| s.to_string()).collect(),
            local_port: 0,
            timeout: STUN_TIMEOUT,
        }
    }
}

impl StunClient {
    /// 创建新的 STUN 客户端
    pub fn new() -> Self {
        Self::default()
    }

    /// 使用指定服务器创建客户端
    pub fn with_servers(servers: Vec<String>) -> Self {
        Self {
            servers,
            local_port: 0,
            timeout: STUN_TIMEOUT,
        }
    }

    /// 设置本地绑定端口
    pub fn with_local_port(mut self, port: u16) -> Self {
        self.local_port = port;
        self
    }

    /// 设置超时时间
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 获取公网地址
    ///
    /// 尝试所有配置的 STUN 服务器，返回第一个成功的结果
    pub async fn get_public_address(&self) -> Result<SocketAddr> {
        for server in &self.servers {
            match query(server, self.timeout).await {
                Ok(addr) => {
                    info!("STUN server {} returned public address: {}", server, addr);
                    return Ok(addr);
                }
                Err(e) => {
                    warn!("STUN server {} failed: {}", server, e);
                }
            }
        }

        Err(SyncthingError::connection("all STUN servers failed"))
    }

    /// 检测 NAT 类型
    ///
    /// 使用同一个 UDP socket 向两个不同的 STUN 服务器查询，
    /// 比较返回的映射地址来判断 NAT 类型。
    ///
    /// 返回 `(NatType, 首选公网地址)`。
    pub async fn detect_nat_type(&self) -> Result<(NatType, Option<SocketAddr>)> {
        if self.servers.len() < 2 {
            warn!(
                "NAT type detection requires at least 2 STUN servers, got {}",
                self.servers.len()
            );
            return Ok((NatType::Unknown, None));
        }

        let bind_addr = SocketAddr::from(([0, 0, 0, 0], self.local_port));
        let socket = UdpSocket::bind(bind_addr)
            .await
            .map_err(|e| SyncthingError::connection(format!("failed to bind UDP socket: {}", e)))?;

        let addr_a = match query_with_socket(&socket, &self.servers[0], self.timeout).await {
            Ok(a) => a,
            Err(e) => {
                warn!("STUN server {} failed: {}", self.servers[0], e);
                return Ok((NatType::Blocked, None));
            }
        };

        let addr_b = match query_with_socket(&socket, &self.servers[1], self.timeout).await {
            Ok(b) => b,
            Err(e) => {
                warn!("STUN server {} failed: {}", self.servers[1], e);
                return Ok((NatType::Unknown, Some(addr_a)));
            }
        };

        let nat_type = if addr_a == addr_b {
            // 两个服务器看到相同的 IP:Port → Cone NAT（或无 NAT）
            NatType::Open
        } else if addr_a.ip() == addr_b.ip() {
            // IP 相同，端口不同 → Restricted Cone（端口受限）
            NatType::Restricted
        } else {
            // IP 都不同 → Symmetric NAT
            NatType::Symmetric
        };

        info!(
            "NAT type detected: {:?} ({} vs {})",
            nat_type, addr_a, addr_b
        );
        Ok((nat_type, Some(addr_a)))
    }

    /// 构建 STUN 绑定请求消息（保留用于测试兼容）
    #[allow(dead_code)]
    fn build_binding_request(&self) -> Result<Vec<u8>> {
        Ok(build_binding_request(new_tx_id()))
    }

    /// 验证地址是否是公网地址
    pub fn is_public_address(addr: &SocketAddr) -> bool {
        match addr.ip() {
            IpAddr::V4(ipv4) => {
                let octets = ipv4.octets();
                // 10.0.0.0/8
                if octets[0] == 10 {
                    return false;
                }
                // 172.16.0.0/12
                if octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31) {
                    return false;
                }
                // 192.168.0.0/16
                if octets[0] == 192 && octets[1] == 168 {
                    return false;
                }
                // 127.0.0.0/8 (loopback)
                if octets[0] == 127 {
                    return false;
                }
                // 169.254.0.0/16 (link-local)
                if octets[0] == 169 && octets[1] == 254 {
                    return false;
                }
                true
            }
            IpAddr::V6(ipv6) => {
                if ipv6.is_loopback() || ipv6.is_unspecified() {
                    return false;
                }
                let segments = ipv6.segments();
                // fe80::/10 link-local
                if (segments[0] & 0xffc0) == 0xfe80 {
                    return false;
                }
                // fc00::/7 unique local
                if (segments[0] & 0xfe00) == 0xfc00 {
                    return false;
                }
                // ff00::/8 multicast
                if (segments[0] & 0xff00) == 0xff00 {
                    return false;
                }
                true
            }
        }
    }
}

/// STUN 地址刷新器
///
/// 定期刷新公网地址，检测 NAT 映射变化
pub struct StunRefresher {
    client: StunClient,
    interval: Duration,
    last_address: std::sync::Arc<tokio::sync::RwLock<Option<SocketAddr>>>,
}

impl StunRefresher {
    /// 创建新的刷新器
    pub fn new(client: StunClient, interval: Duration) -> Self {
        Self {
            client,
            interval,
            last_address: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// 启动定期刷新任务
    pub async fn start<F>(&self, mut on_change: F) -> Result<()>
    where
        F: FnMut(SocketAddr) + Send + 'static,
    {
        let mut interval = tokio::time::interval(self.interval);
        let client = self.client.clone();
        let last_address = self.last_address.clone();

        loop {
            interval.tick().await;

            match client.get_public_address().await {
                Ok(new_addr) => {
                    let mut last = last_address.write().await;

                    if last.map(|old| old != new_addr).unwrap_or(true) {
                        info!("Public address changed: {:?} -> {}", last, new_addr);
                        *last = Some(new_addr);
                        on_change(new_addr);
                    }
                }
                Err(e) => {
                    warn!("Failed to refresh public address: {}", e);
                }
            }
        }
    }

    /// 获取最后已知的公网地址
    pub async fn last_address(&self) -> Option<SocketAddr> {
        *self.last_address.read().await
    }
}


#[cfg(test)]
mod tests;
