//! PCP port mapping implementation.
//!
//! (reference: Tailscale net/portmapper/pcp.go)

/// PCP version.
pub const PCP_VERSION: u8 = 2;

/// PCP default port.
pub const PCP_DEFAULT_PORT: u16 = 5351;

/// PCP mapping lifetime in seconds.
pub const PCP_MAP_LIFETIME_SEC: u32 = 7200;

/// PCP opcodes.
pub const PCP_OP_REPLY: u8 = 0x80;
pub const PCP_OP_ANNOUNCE: u8 = 0;
pub const PCP_OP_MAP: u8 = 1;

/// PCP protocol numbers.
pub const PCP_UDP_MAPPING: u8 = 17;
pub const PCP_TCP_MAPPING: u8 = 6;

/// PCP result code.
pub type PcpResultCode = u8;
pub const PCP_CODE_OK: PcpResultCode = 0;
pub const PCP_CODE_NOT_AUTHORIZED: PcpResultCode = 2;
pub const PCP_CODE_ADDRESS_MISMATCH: PcpResultCode = 12;

/// PCP response.
#[derive(Debug, Clone)]
pub struct PcpResponse {
    /// Opcode with the reply bit set.
    pub op_code: u8,
    /// Result code.
    pub result_code: PcpResultCode,
    /// Granted lifetime in seconds.
    pub lifetime: u32,
}

/// PCP mapping state.
#[derive(Debug, Clone)]
pub struct PcpMappingState {
    /// Gateway address.
    pub gateway: std::net::SocketAddr,
    /// External port.
    pub external_port: u16,
    /// Internal port.
    pub internal_port: u16,
    /// Local IP used in the request.
    pub my_ip: std::net::Ipv4Addr,
}

use std::time::Duration;
use syncthing_core::Result;
use tokio::net::UdpSocket;
use tokio::time::timeout;

/// Encodes an IPv4 address as an IPv4-mapped IPv6 address (16 bytes).
fn encode_ipv4_mapped(ip: std::net::Ipv4Addr) -> [u8; 16] {
    let octets = ip.octets();
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, octets[0], octets[1], octets[2], octets[3],
    ]
}

/// Builds a PCP ANNOUNCE request packet (24-byte header).
pub fn build_pcp_announce_request(my_ip: std::net::Ipv4Addr) -> Vec<u8> {
    let mut pkt = vec![0u8; 24];
    pkt[0] = PCP_VERSION;
    pkt[1] = PCP_OP_ANNOUNCE;
    // pkt[2..4] reserved = 0; pkt[4..8] lifetime = 0 for announce.
    let ip_bytes = encode_ipv4_mapped(my_ip);
    pkt[8..24].copy_from_slice(&ip_bytes);
    pkt
}

/// Builds a PCP MAP request packet (24-byte header + 36 bytes opcode data = 60 bytes).
pub fn build_pcp_request_mapping_packet(
    my_ip: std::net::Ipv4Addr,
    local_port: u16,
    prev_port: u16,
    lifetime_sec: u32,
    prev_external_ip: std::net::Ipv4Addr,
) -> Vec<u8> {
    let mut pkt = vec![0u8; 60];
    // Common header (24 bytes); pkt[2..4] reserved.
    pkt[0] = PCP_VERSION;
    pkt[1] = PCP_OP_MAP;
    pkt[4..8].copy_from_slice(&lifetime_sec.to_be_bytes());
    let ip_bytes = encode_ipv4_mapped(my_ip);
    pkt[8..24].copy_from_slice(&ip_bytes);

    // Opcode-specific data (36 bytes); pkt[24..36] nonce = 0; pkt[37..40] reserved.
    pkt[36] = PCP_UDP_MAPPING;
    pkt[40..42].copy_from_slice(&local_port.to_be_bytes());
    pkt[42..44].copy_from_slice(&prev_port.to_be_bytes());
    let ext_ip_bytes = encode_ipv4_mapped(prev_external_ip);
    pkt[44..60].copy_from_slice(&ext_ip_bytes);

    pkt
}

/// Parses a PCP response (24-byte common header + optional opcode data).
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
    // RFC 6887: response header lifetime at offset 4..8
    let lifetime = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);

    Some(PcpResponse {
        op_code,
        result_code,
        lifetime,
    })
}

/// Probes whether a PCP gateway responds.
pub async fn probe_gateway(gateway: std::net::SocketAddr, my_ip: std::net::Ipv4Addr) -> bool {
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0").await else {
        return false;
    };
    let pkt = build_pcp_announce_request(my_ip);
    if socket.send_to(&pkt, gateway).await.is_err() {
        return false;
    }
    let mut buf = [0u8; 24];
    match timeout(Duration::from_millis(250), socket.recv_from(&mut buf)).await {
        Ok(Ok((len, _))) => parse_pcp_response(&buf[..len])
            .map(|r| r.result_code == PCP_CODE_OK)
            .unwrap_or(false),
        _ => false,
    }
}

/// Allocates a PCP port mapping.
///
/// # Errors
/// Returns an error on bind/send failure, timeout, non-OK result code, or a
/// truncated response.
pub async fn allocate_port(
    gateway: std::net::SocketAddr,
    local_port: u16,
    my_ip: std::net::Ipv4Addr,
) -> Result<(std::net::SocketAddr, PcpMappingState)> {
    let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(|e| {
        syncthing_core::SyncthingError::connection(format!("PCP bind failed: {}", e))
    })?;

    let pkt = build_pcp_request_mapping_packet(
        my_ip,
        local_port,
        local_port,
        PCP_MAP_LIFETIME_SEC,
        std::net::Ipv4Addr::UNSPECIFIED,
    );
    socket.send_to(&pkt, gateway).await.map_err(|e| {
        syncthing_core::SyncthingError::connection(format!("PCP send failed: {}", e))
    })?;

    let mut buf = [0u8; 60];
    let (len, _) = timeout(Duration::from_secs(5), socket.recv_from(&mut buf))
        .await
        .map_err(|_| syncthing_core::SyncthingError::connection("PCP mapping timeout"))?
        .map_err(|e| {
            syncthing_core::SyncthingError::connection(format!("PCP recv failed: {}", e))
        })?;

    let resp = parse_pcp_response(&buf[..len])
        .ok_or_else(|| syncthing_core::SyncthingError::connection("PCP invalid response"))?;

    if resp.result_code != PCP_CODE_OK {
        return Err(syncthing_core::SyncthingError::connection(format!(
            "PCP error code: {}",
            resp.result_code
        )));
    }

    if len < 60 {
        return Err(syncthing_core::SyncthingError::connection(
            "PCP MAP response too short",
        ));
    }

    // MAP response layout: external port at offset 42..44, external IP at 44..60.
    let external_port = u16::from_be_bytes([buf[42], buf[43]]);
    let external_ip =
        std::net::Ipv4Addr::new(buf[44 + 12], buf[44 + 13], buf[44 + 14], buf[44 + 15]);

    let external_addr = std::net::SocketAddr::from((external_ip, external_port));
    let state = PcpMappingState {
        gateway,
        external_port,
        internal_port: local_port,
        my_ip,
    };

    Ok((external_addr, state))
}
