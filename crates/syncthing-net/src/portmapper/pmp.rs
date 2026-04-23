//! NAT-PMP 端口映射实现
//!
//! 参考来源: Tailscale net/portmapper/portmapper.go

use std::net::{Ipv4Addr, SocketAddr};

/// NAT-PMP 默认端口
pub const PMP_DEFAULT_PORT: u16 = 5351;

/// NAT-PMP 映射持续时间（秒）
pub const PMP_MAP_LIFETIME_SEC: u32 = 7200;

/// NAT-PMP 版本号
pub const PMP_VERSION: u8 = 0;

/// NAT-PMP 操作码
pub const PMP_OP_MAP_PUBLIC_ADDR: u8 = 0;
pub const PMP_OP_MAP_UDP: u8 = 1;
pub const PMP_OP_REPLY: u8 = 0x80;

/// NAT-PMP 结果码
pub type PmpResultCode = u16;
pub const PMP_CODE_OK: PmpResultCode = 0;
pub const PMP_CODE_UNSUPPORTED_VERSION: PmpResultCode = 1;
pub const PMP_CODE_NOT_AUTHORIZED: PmpResultCode = 2;
pub const PMP_CODE_NETWORK_FAILURE: PmpResultCode = 3;
pub const PMP_CODE_OUT_OF_RESOURCES: PmpResultCode = 4;
pub const PMP_CODE_UNSUPPORTED_OPCODE: PmpResultCode = 5;

/// NAT-PMP 响应
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmpResponse {
    pub op_code: u8,
    pub result_code: PmpResultCode,
    pub seconds_since_epoch: u32,
    pub mapping_valid_seconds: u32,
    pub internal_port: u16,
    pub external_port: u16,
    pub public_addr: Option<Ipv4Addr>,
}

/// PMP 映射状态
#[derive(Debug, Clone)]
pub struct PmpMappingState {
    pub gateway: SocketAddr,
    pub external_port: u16,
    pub internal_port: u16,
}

/// 构建 NAT-PMP 请求映射包
pub fn build_pmp_request_mapping_packet(local_port: u16, prev_port: u16, lifetime_sec: u32) -> Vec<u8> {
    let mut pkt = vec![0u8; 12];
    pkt[0] = PMP_VERSION;
    pkt[1] = PMP_OP_MAP_UDP;
    pkt[4..6].copy_from_slice(&local_port.to_be_bytes());
    pkt[6..8].copy_from_slice(&prev_port.to_be_bytes());
    pkt[8..12].copy_from_slice(&lifetime_sec.to_be_bytes());
    pkt
}

/// 解析 NAT-PMP 响应
pub fn parse_pmp_response(pkt: &[u8]) -> Option<PmpResponse> {
    if pkt.len() < 12 {
        return None;
    }
    let ver = pkt[0];
    if ver != PMP_VERSION {
        return None;
    }

    let op_code = pkt[1];
    let result_code = u16::from_be_bytes([pkt[2], pkt[3]]);
    let seconds_since_epoch = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);

    let mut res = PmpResponse {
        op_code,
        result_code,
        seconds_since_epoch,
        mapping_valid_seconds: 0,
        internal_port: 0,
        external_port: 0,
        public_addr: None,
    };

    if op_code == PMP_OP_REPLY | PMP_OP_MAP_UDP {
        if pkt.len() != 16 {
            return None;
        }
        res.internal_port = u16::from_be_bytes([pkt[8], pkt[9]]);
        res.external_port = u16::from_be_bytes([pkt[10], pkt[11]]);
        res.mapping_valid_seconds = u32::from_be_bytes([pkt[12], pkt[13], pkt[14], pkt[15]]);
    } else if op_code == PMP_OP_REPLY | PMP_OP_MAP_PUBLIC_ADDR {
        if pkt.len() != 12 {
            return None;
        }
        let addr = Ipv4Addr::new(pkt[8], pkt[9], pkt[10], pkt[11]);
        if !addr.is_unspecified() {
            res.public_addr = Some(addr);
        }
    }

    Some(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_pmp_request_mapping_packet() {
        let pkt = build_pmp_request_mapping_packet(22000, 22000, 7200);
        assert_eq!(pkt.len(), 12);
        assert_eq!(pkt[0], 0); // version
        assert_eq!(pkt[1], 1); // op = map UDP
        assert_eq!(&pkt[2..4], &[0, 0]); // reserved in request
        assert_eq!(&pkt[4..6], &[0x55, 0xF0]); // local_port = 22000
        assert_eq!(&pkt[6..8], &[0x55, 0xF0]); // prev_port = 22000
        assert_eq!(&pkt[8..12], &[0, 0, 0x1C, 0x20]); // lifetime = 7200
    }

    #[test]
    fn test_parse_pmp_response_map_udp() {
        // 构造一个有效的 PMP UDP 映射响应
        let mut pkt = vec![0u8; 16];
        pkt[0] = 0; // version
        pkt[1] = 0x81; // reply | map UDP
        pkt[2..4].copy_from_slice(&0u16.to_be_bytes()); // result code = 0
        pkt[4..8].copy_from_slice(&1234u32.to_be_bytes()); // seconds since epoch
        pkt[8..10].copy_from_slice(&22000u16.to_be_bytes()); // internal port
        pkt[10..12].copy_from_slice(&22000u16.to_be_bytes()); // external port
        pkt[12..16].copy_from_slice(&7200u32.to_be_bytes()); // mapping valid seconds

        let res = parse_pmp_response(&pkt).unwrap();
        assert_eq!(res.op_code, 0x81);
        assert_eq!(res.result_code, 0);
        assert_eq!(res.seconds_since_epoch, 1234);
        assert_eq!(res.internal_port, 22000);
        assert_eq!(res.external_port, 22000);
        assert_eq!(res.mapping_valid_seconds, 7200);
        assert_eq!(res.public_addr, None);
    }

    #[test]
    fn test_parse_pmp_response_public_addr() {
        let mut pkt = vec![0u8; 12];
        pkt[0] = 0; // version
        pkt[1] = 0x80; // reply | public addr
        pkt[2..4].copy_from_slice(&0u16.to_be_bytes());
        pkt[4..8].copy_from_slice(&5678u32.to_be_bytes());
        pkt[8] = 192;
        pkt[9] = 168;
        pkt[10] = 1;
        pkt[11] = 1;

        let res = parse_pmp_response(&pkt).unwrap();
        assert_eq!(res.op_code, 0x80);
        assert_eq!(res.result_code, 0);
        assert_eq!(res.seconds_since_epoch, 5678);
        assert_eq!(res.public_addr, Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn test_parse_pmp_response_invalid() {
        // 长度不足
        assert!(parse_pmp_response(&[0, 1]).is_none());
        // 版本错误
        assert!(parse_pmp_response(&[1; 12]).is_none());
        // UDP 映射响应长度错误
        let mut pkt = vec![0u8; 14];
        pkt[0] = 0;
        pkt[1] = 0x81;
        assert!(parse_pmp_response(&pkt).is_none());
    }
}
