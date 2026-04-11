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
pub struct PcpMappingState;

/// 构建 PCP ANNOUNCE 请求包
///
/// TODO: 完整实现 PCP 协议
pub fn build_pcp_announce_request(_my_ip: std::net::Ipv4Addr) -> Vec<u8> {
    // TODO: 实现 PCP ANNOUNCE 请求构造
    todo!("PCP announce request not yet implemented")
}

/// 构建 PCP MAP 请求包
///
/// TODO: 完整实现 PCP 协议
pub fn build_pcp_request_mapping_packet(
    _my_ip: std::net::Ipv4Addr,
    _local_port: u16,
    _prev_port: u16,
    _lifetime_sec: u32,
    _prev_external_ip: std::net::Ipv4Addr,
) -> Vec<u8> {
    // TODO: 实现 PCP MAP 请求构造
    todo!("PCP mapping request not yet implemented")
}

/// 解析 PCP 响应
///
/// TODO: 完整实现 PCP 响应解析
pub fn parse_pcp_response(_pkt: &[u8]) -> Option<PcpResponse> {
    // TODO: 实现 PCP 响应解析
    todo!("PCP response parsing not yet implemented")
}
