//! DERP（Detoured Encrypted Routing Protocol）协议定义
//!
//! DERP 是 Tailscale 风格的中继协议，用于两台 NAT 后设备无法直连时的
//! 数据包转发。协议极简，帧格式如下：
//!
//! ```text
//! [4 bytes: payload length (big-endian)]
//! [1 byte: frame type]
//! [n bytes: payload]
//! ```
//!
//! 参考: https://tailscale.com/kb/1232/derp-servers

use bytes::{Buf, BufMut, BytesMut};
use syncthing_core::{DeviceId, Result, SyncthingError};

/// 当前协议版本
pub const PROTOCOL_VERSION: u8 = 1;

/// 最大帧大小（防止内存耗尽）
pub const MAX_FRAME_SIZE: u32 = 1_000_000; // 1 MB

/// DERP 帧类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// 客户端注册信息
    ClientInfo = 0x01,
    /// 服务器响应信息
    ServerInfo = 0x02,
    /// 客户端请求转发数据包
    SendPacket = 0x03,
    /// 服务器转发来的数据包
    RecvPacket = 0x04,
    /// 心跳保活
    KeepAlive = 0x05,
    /// 关闭与某对等端的连接（可选）
    ClosePeer = 0x06,
}

impl FrameType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(FrameType::ClientInfo),
            0x02 => Some(FrameType::ServerInfo),
            0x03 => Some(FrameType::SendPacket),
            0x04 => Some(FrameType::RecvPacket),
            0x05 => Some(FrameType::KeepAlive),
            0x06 => Some(FrameType::ClosePeer),
            _ => None,
        }
    }
}

/// DERP 帧
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// 客户端注册
    ClientInfo {
        device_id: DeviceId,
        version: u8,
    },
    /// 服务器信息
    ServerInfo {
        version: u8,
    },
    /// 发送数据包到目标设备
    SendPacket {
        target: DeviceId,
        payload: Vec<u8>,
    },
    /// 收到来自某设备的数据包
    RecvPacket {
        from: DeviceId,
        payload: Vec<u8>,
    },
    /// 心跳
    KeepAlive,
    /// 关闭对等端
    ClosePeer {
        target: DeviceId,
    },
}

impl Frame {
    /// 编码帧为字节（包含长度前缀）
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::new();

        // 先写入类型和 payload，再回填长度
        let type_byte = self.frame_type() as u8;
        buf.put_u8(type_byte);

        match self {
            Frame::ClientInfo { device_id, version } => {
                buf.extend_from_slice(&device_id.0);
                buf.put_u8(*version);
            }
            Frame::ServerInfo { version } => {
                buf.put_u8(*version);
            }
            Frame::SendPacket { target, payload } => {
                buf.extend_from_slice(&target.0);
                buf.put_u32(payload.len() as u32);
                buf.extend_from_slice(payload);
            }
            Frame::RecvPacket { from, payload } => {
                buf.extend_from_slice(&from.0);
                buf.put_u32(payload.len() as u32);
                buf.extend_from_slice(payload);
            }
            Frame::KeepAlive => {}
            Frame::ClosePeer { target } => {
                buf.extend_from_slice(&target.0);
            }
        }

        // 回填长度前缀（payload 长度 = buf.len() - 4，但因为我们在开头留了位置，
        // 实际上需要先计算总长度再 put_u32）
        let payload_len = buf.len() as u32;
        let mut out = Vec::with_capacity(4 + buf.len());
        out.put_u32(payload_len);
        out.extend_from_slice(&buf);
        out
    }

    /// 从字节缓冲区解码帧。
    ///
    /// `buf` 必须至少包含一个完整帧（长度前缀 + payload）。
    /// 返回解码后的帧和消耗的原始字节数。
    pub fn decode(buf: &mut BytesMut) -> Result<Option<(Self, usize)>> {
        if buf.len() < 4 {
            return Ok(None);
        }

        // Peek length without consuming bytes
        let payload_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if payload_len > MAX_FRAME_SIZE as usize {
            return Err(SyncthingError::protocol(format!(
                "DERP frame too large: {} > {}",
                payload_len, MAX_FRAME_SIZE
            )));
        }

        let total_len = 4 + payload_len;
        if buf.len() < total_len {
            return Ok(None);
        }

        // 现在确认数据足够，消耗长度前缀
        buf.advance(4);

        if payload_len < 1 {
            return Err(SyncthingError::protocol("DERP frame too short"));
        }

        let frame_type = FrameType::from_u8(buf.get_u8())
            .ok_or_else(|| SyncthingError::protocol(format!("unknown DERP frame type")))?;

        let frame = match frame_type {
            FrameType::ClientInfo => {
                if payload_len < 1 + 32 {
                    return Err(SyncthingError::protocol("ClientInfo frame too short"));
                }
                let mut device_id_bytes = [0u8; 32];
                buf.copy_to_slice(&mut device_id_bytes);
                let version = buf.get_u8();
                Frame::ClientInfo {
                    device_id: DeviceId(device_id_bytes),
                    version,
                }
            }
            FrameType::ServerInfo => {
                if payload_len < 2 {
                    return Err(SyncthingError::protocol("ServerInfo frame too short"));
                }
                let version = buf.get_u8();
                Frame::ServerInfo { version }
            }
            FrameType::SendPacket => {
                if payload_len < 1 + 32 + 4 {
                    return Err(SyncthingError::protocol("SendPacket frame too short"));
                }
                let mut target_bytes = [0u8; 32];
                buf.copy_to_slice(&mut target_bytes);
                let data_len = buf.get_u32() as usize;
                if payload_len < 1 + 32 + 4 + data_len {
                    return Err(SyncthingError::protocol("SendPacket payload truncated"));
                }
                let mut payload = vec![0u8; data_len];
                buf.copy_to_slice(&mut payload);
                Frame::SendPacket {
                    target: DeviceId(target_bytes),
                    payload,
                }
            }
            FrameType::RecvPacket => {
                if payload_len < 1 + 32 + 4 {
                    return Err(SyncthingError::protocol("RecvPacket frame too short"));
                }
                let mut from_bytes = [0u8; 32];
                buf.copy_to_slice(&mut from_bytes);
                let data_len = buf.get_u32() as usize;
                if payload_len < 1 + 32 + 4 + data_len {
                    return Err(SyncthingError::protocol("RecvPacket payload truncated"));
                }
                let mut payload = vec![0u8; data_len];
                buf.copy_to_slice(&mut payload);
                Frame::RecvPacket {
                    from: DeviceId(from_bytes),
                    payload,
                }
            }
            FrameType::KeepAlive => Frame::KeepAlive,
            FrameType::ClosePeer => {
                if payload_len < 1 + 32 {
                    return Err(SyncthingError::protocol("ClosePeer frame too short"));
                }
                let mut target_bytes = [0u8; 32];
                buf.copy_to_slice(&mut target_bytes);
                Frame::ClosePeer {
                    target: DeviceId(target_bytes),
                }
            }
        };

        Ok(Some((frame, total_len)))
    }

    fn frame_type(&self) -> FrameType {
        match self {
            Frame::ClientInfo { .. } => FrameType::ClientInfo,
            Frame::ServerInfo { .. } => FrameType::ServerInfo,
            Frame::SendPacket { .. } => FrameType::SendPacket,
            Frame::RecvPacket { .. } => FrameType::RecvPacket,
            Frame::KeepAlive => FrameType::KeepAlive,
            Frame::ClosePeer { .. } => FrameType::ClosePeer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn test_frame_roundtrip() {
        let frame = Frame::ClientInfo {
            device_id: DeviceId::default(),
            version: PROTOCOL_VERSION,
        };
        let encoded = frame.encode();
        let mut buf = BytesMut::from(&encoded[..]);
        let (decoded, consumed) = Frame::decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, frame);
        assert_eq!(consumed, encoded.len());
    }

    #[test]
    fn test_send_packet_roundtrip() {
        let frame = Frame::SendPacket {
            target: DeviceId::default(),
            payload: vec![1, 2, 3, 4, 5],
        };
        let encoded = frame.encode();
        let mut buf = BytesMut::from(&encoded[..]);
        let (decoded, _) = Frame::decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn test_keep_alive_roundtrip() {
        let frame = Frame::KeepAlive;
        let encoded = frame.encode();
        let mut buf = BytesMut::from(&encoded[..]);
        let (decoded, _) = Frame::decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn test_decode_incomplete() {
        let mut buf = BytesMut::from(&[0u8; 2][..]);
        assert!(Frame::decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn test_frame_too_large() {
        let mut buf = BytesMut::new();
        buf.put_u32(MAX_FRAME_SIZE + 1);
        buf.put_u8(FrameType::KeepAlive as u8);
        let result = Frame::decode(&mut buf);
        assert!(result.is_err());
    }
}
