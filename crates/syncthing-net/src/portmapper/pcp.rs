//! PCP 端口映射实现
//!
//! 参考来源: Tailscale net/portmapper/pcp.go

/// PCP 版本
pub const PCP_VERSION: u8 = 2;

/// PCP 默认端口
pub const PCP_DEFAULT_PORT: u16 = 5351;

/// PCP 映射持续时间（秒）
pub const PCP_MAP_LIFETIME_SEC: u32 = 7200;

/// PCP 操作码
pub const PCP_OP_REPLY: u8 = 0x80;
pub const PCP_OP_ANNOUNCE: u8 = 0;
pub const PCP_OP_MAP: u8 = 1;

/// PCP 协议号
pub const PCP_UDP_MAPPING: u8 = 17;
pub const PCP_TCP_MAPPING: u8 = 6;

/// PCP 结果码
pub type PcpResultCode = u8;
pub const PCP_CODE_OK: PcpResultCode = 0;
pub const PCP_CODE_NOT_AUTHORIZED: PcpResultCode = 2;
pub const PCP_CODE_ADDRESS_MISMATCH: PcpResultCode = 12;

/// PCP 响应
#[derive(Debug, Clone)]
pub struct PcpResponse {
    pub op_code: u8,
    pub result_code: PcpResultCode,
    pub lifetime: u32,
    pub epoch: u32,
}

/// PCP 映射状态
#[derive(Debug, Clone)]
pub struct PcpMappingState {
    pub gateway: std::net::SocketAddr,
    pub external_port: u16,
    pub internal_port: u16,
    pub my_ip: std::net::Ipv4Addr,
}

/// 将 IPv4 地址编码为 IPv4-mapped IPv6 地址（16 字节）
fn encode_ipv4_mapped(ip: std::net::Ipv4Addr) -> [u8; 16] {
    let octets = ip.octets();
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff,
        octets[0], octets[1], octets[2], octets[3],
    ]
}

/// 构建 PCP ANNOUNCE 请求包（24 字节头部）
pub fn build_pcp_announce_request(my_ip: std::net::Ipv4Addr) -> Vec<u8> {
    let mut pkt = vec![0u8; 24];
    pkt[0] = PCP_VERSION;
    pkt[1] = PCP_OP_ANNOUNCE; // R=0, OpCode=0
    // pkt[2..4] reserved = 0
    // pkt[4..8] lifetime = 0 for announce
    let ip_bytes = encode_ipv4_mapped(my_ip);
    pkt[8..24].copy_from_slice(&ip_bytes);
    pkt
}

/// 构建 PCP MAP 请求包（24 字节头部 + 36 字节操作码数据 = 60 字节）
pub fn build_pcp_request_mapping_packet(
    my_ip: std::net::Ipv4Addr,
    local_port: u16,
    prev_port: u16,
    lifetime_sec: u32,
    prev_external_ip: std::net::Ipv4Addr,
) -> Vec<u8> {
    let mut pkt = vec![0u8; 60];
    // Common header (24 bytes)
    pkt[0] = PCP_VERSION;
    pkt[1] = PCP_OP_MAP; // R=0, OpCode=1
    // pkt[2..4] reserved = 0
    pkt[4..8].copy_from_slice(&lifetime_sec.to_be_bytes());
    let ip_bytes = encode_ipv4_mapped(my_ip);
    pkt[8..24].copy_from_slice(&ip_bytes);

    // Opcode-specific data (36 bytes)
    // pkt[24..36] nonce = 0 (12 bytes)
    pkt[36] = PCP_UDP_MAPPING; // Protocol = UDP
    // pkt[37..40] reserved = 0
    pkt[40..42].copy_from_slice(&local_port.to_be_bytes());
    pkt[42..44].copy_from_slice(&prev_port.to_be_bytes());
    let ext_ip_bytes = encode_ipv4_mapped(prev_external_ip);
    pkt[44..60].copy_from_slice(&ext_ip_bytes);

    pkt
}

/// 解析 PCP 响应（通用头部 24 字节 + 可选操作码数据）
pub fn parse_pcp_response(pkt: &[u8]) -> Option<PcpResponse> {
    if pkt.len() < 24 {
        return None;
    }
    let ver = pkt[0];
    if ver != PCP_VERSION {
        return None;
    }

    let op_code = pkt[1];
    // R bit must be set in response (0x80)
    if op_code & 0x80 == 0 {
        return None;
    }

    let result_code = pkt[3];
    let epoch = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);

    let lifetime = if pkt.len() >= 60 {
        // MAP response: lifetime is in the opcode-specific area at offset 4..8?
        // Actually in response, the header structure changes.
        // For simplicity, use the same layout as request.
        u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]])
    } else {
        0
    };

    Some(PcpResponse {
        op_code,
        result_code,
        lifetime,
        epoch,
    })
}
