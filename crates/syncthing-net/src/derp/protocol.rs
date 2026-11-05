//! DERP (Detoured Encrypted Routing Protocol) protocol definition.
//!
//! DERP is a Tailscale-style relay protocol forwarding packets between two
//! NATed devices that cannot connect directly. The protocol is minimal:
//!
//! ```text
//! [4 bytes: payload length (big-endian)]
//! [1 byte: frame type]
//! [n bytes: payload]
//! ```
//!
//! (reference: <https://tailscale.com/kb/1232/derp-servers>)

use bytes::{Buf, BufMut, BytesMut};
use syncthing_core::{DeviceId, Result, SyncthingError};

/// Current protocol version.
pub const PROTOCOL_VERSION: u8 = 1;

/// Maximum frame size (prevents memory exhaustion).
pub const MAX_FRAME_SIZE: u32 = 1_000_000; // 1 MB

/// DERP frame type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// Client registration info.
    ClientInfo = 0x01,
    /// Server response info.
    ServerInfo = 0x02,
    /// Client request to forward a packet.
    SendPacket = 0x03,
    /// Packet forwarded by the server.
    RecvPacket = 0x04,
    /// Keepalive heartbeat.
    KeepAlive = 0x05,
    /// Close the connection to a peer (optional).
    ClosePeer = 0x06,
}

impl FrameType {
    /// Converts a raw type byte to a `FrameType`, or `None` if unknown.
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

/// DERP frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Client registration.
    ClientInfo { device_id: DeviceId, version: u8 },
    /// Server info.
    ServerInfo { version: u8 },
    /// Send a packet to the target device.
    SendPacket { target: DeviceId, payload: Vec<u8> },
    /// Packet received from a device.
    RecvPacket { from: DeviceId, payload: Vec<u8> },
    /// Heartbeat.
    KeepAlive,
    /// Close a peer.
    ClosePeer { target: DeviceId },
}

impl Frame {
    /// Encodes the frame to bytes (including the length prefix).
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = BytesMut::new();

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

        let payload_len = buf.len() as u32;
        let mut out = Vec::with_capacity(4 + buf.len());
        out.put_u32(payload_len);
        out.extend_from_slice(&buf);
        out
    }

    /// Decodes a frame from the byte buffer.
    ///
    /// `buf` must contain at least one complete frame (length prefix + payload).
    /// Returns the decoded frame and the number of raw bytes consumed, or `None`
    /// when more data is needed.
    ///
    /// # Errors
    /// Returns an error on oversized frames, unknown frame types, or malformed
    /// payloads.
    pub fn decode(buf: &mut BytesMut) -> Result<Option<(Self, usize)>> {
        if buf.len() < 4 {
            return Ok(None);
        }

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

        buf.advance(4);

        if payload_len < 1 {
            return Err(SyncthingError::protocol("DERP frame too short"));
        }

        let frame_type = FrameType::from_u8(buf.get_u8())
            .ok_or_else(|| SyncthingError::protocol("unknown DERP frame type".to_string()))?;

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
