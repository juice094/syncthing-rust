//! BEP protocol message definitions.
//!
//! Mirrors the message type layout of syncthing/lib/protocol/*.go.

use bytes::Bytes;
use prost::Message;
use serde::Serialize;

use bep_protocol::messages::Hello as BepHello;
use syncthing_core::DeviceId;

/// BEP magic number (0x2EA7D90B).
pub const BEP_MAGIC: u32 = 0x2EA7D90B;

/// BEP magic bytes in network byte order.
pub const BEP_MAGIC_BYTES: &[u8] = &BEP_MAGIC.to_be_bytes();

/// BEP message type.
///
/// Aligned with the Go-side BEP MessageType:
/// ClusterConfig=0, Index=1, IndexUpdate=2, Request=3, Response=4,
/// DownloadProgress=5, Ping=6, Close=7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MessageType {
    /// Cluster configuration.
    ClusterConfig = 0,
    /// Index message.
    Index = 1,
    /// Index update.
    IndexUpdate = 2,
    /// Block request.
    Request = 3,
    /// Block response.
    Response = 4,
    /// Download progress.
    DownloadProgress = 5,
    /// Ping.
    Ping = 6,
    /// Connection close.
    Close = 7,
}

impl MessageType {
    /// Converts a raw BEP type value to a `MessageType`, or `None` if unknown.
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(MessageType::ClusterConfig),
            1 => Some(MessageType::Index),
            2 => Some(MessageType::IndexUpdate),
            3 => Some(MessageType::Request),
            4 => Some(MessageType::Response),
            5 => Some(MessageType::DownloadProgress),
            6 => Some(MessageType::Ping),
            7 => Some(MessageType::Close),
            _ => None,
        }
    }
}

/// BEP message header.
#[derive(Debug, Clone)]
pub struct MessageHeader {
    /// Message type.
    pub message_type: MessageType,
    /// Message ID (used to match request/response pairs).
    pub message_id: u32,
    /// Whether the payload is LZ4-compressed.
    pub compressed: bool,
}

impl MessageHeader {
    /// Encodes the header into a BEP Header protobuf.
    pub fn to_bep_header(&self) -> bep_protocol::messages::Header {
        bep_protocol::messages::Header {
            r#type: self.message_type as i32,
            compression: if self.compressed {
                bep_protocol::messages::MessageCompression::Lz4 as i32
            } else {
                bep_protocol::messages::MessageCompression::None as i32
            },
        }
    }

    /// Builds a `MessageHeader` from a BEP Header protobuf, or `None` for unknown types.
    pub fn from_bep_header(header: &bep_protocol::messages::Header) -> Option<Self> {
        let msg_type = match header.r#type {
            0 => MessageType::ClusterConfig,
            1 => MessageType::Index,
            2 => MessageType::IndexUpdate,
            3 => MessageType::Request,
            4 => MessageType::Response,
            5 => MessageType::DownloadProgress,
            6 => MessageType::Ping,
            7 => MessageType::Close,
            _ => return None,
        };
        Some(Self {
            message_type: msg_type,
            message_id: 0,
            compressed: header.compression
                == bep_protocol::messages::MessageCompression::Lz4 as i32,
        })
    }
}

/// BEP Hello message.
#[derive(Debug, Clone)]
pub struct HelloMessage {
    /// Device identifier.
    pub device_id: DeviceId,
    /// Human-readable device name.
    pub device_name: String,
    /// Client name (e.g. `syncthing-rust`).
    pub client_name: String,
    /// Client version string.
    pub client_version: String,
    /// Supported protocol identifiers.
    pub protocols: Vec<String>,
}

impl HelloMessage {
    /// Creates a new Hello message for the given device.
    pub fn new(device_id: DeviceId) -> Self {
        Self {
            device_id,
            device_name: String::new(),
            client_name: "syncthing-rust".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            protocols: vec!["bep/1.0".to_string()],
        }
    }

    /// Encodes the Hello into wire bytes.
    ///
    /// Wire format: `[magic: u32 = 0x2EA7D90B][length: u16][protobuf Hello bytes]`.
    ///
    /// # Errors
    /// Returns an error string if the protobuf encoding exceeds the maximum
    /// Hello size allowed by the BEP handshake.
    pub fn encode(&self) -> std::result::Result<Bytes, String> {
        let bep_hello = BepHello {
            device_name: self.device_name.clone(),
            client_name: self.client_name.clone(),
            client_version: self.client_version.clone(),
            num_connections: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        };

        let msg_bytes = bep_hello.encode_to_vec();
        if msg_bytes.len() > bep_protocol::handshake::MAX_HELLO_SIZE {
            return Err(format!(
                "Hello too large: {} > {}",
                msg_bytes.len(),
                bep_protocol::handshake::MAX_HELLO_SIZE
            ));
        }

        let mut buf = Vec::with_capacity(6 + msg_bytes.len());
        buf.extend_from_slice(&bep_protocol::handshake::HELLO_MAGIC.to_be_bytes());
        buf.extend_from_slice(&(msg_bytes.len() as u16).to_be_bytes());
        buf.extend_from_slice(&msg_bytes);

        Ok(Bytes::from(buf))
    }

    /// Decodes a Hello from wire bytes.
    ///
    /// Wire format: `[magic: u32 = 0x2EA7D90B][length: u16][protobuf Hello bytes]`.
    ///
    /// # Errors
    /// Returns an error string if the data is too short, carries a wrong magic,
    /// is truncated, or fails protobuf decoding.
    pub fn decode(data: &[u8]) -> std::result::Result<Self, String> {
        if data.len() < 6 {
            return Err("Hello data too short".to_string());
        }

        let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        if magic != bep_protocol::handshake::HELLO_MAGIC {
            return Err(format!(
                "invalid hello magic: expected 0x{:08X}, got 0x{:08X}",
                bep_protocol::handshake::HELLO_MAGIC,
                magic
            ));
        }

        let len = u16::from_be_bytes([data[4], data[5]]) as usize;
        if data.len() < 6 + len {
            return Err("truncated hello data".to_string());
        }

        let bep_hello = BepHello::decode(&data[6..6 + len])
            .map_err(|e| format!("failed to decode hello: {}", e))?;

        Ok(Self {
            device_id: DeviceId::default(),
            device_name: bep_hello.device_name,
            client_name: bep_hello.client_name,
            client_version: bep_hello.client_version,
            protocols: vec!["bep/1.0".to_string()],
        })
    }
}

impl Default for HelloMessage {
    fn default() -> Self {
        Self::new(DeviceId::default())
    }
}

/// BEP Request message.
#[derive(Debug, Clone, Serialize)]
pub struct RequestMessage {
    /// Request ID (matched against the Response ID).
    pub id: i32,
    /// Folder identifier.
    pub folder: String,
    /// File name within the folder.
    pub name: String,
    /// Byte offset of the requested block.
    pub offset: i64,
    /// Block size in bytes.
    pub size: i32,
    /// Expected block hash.
    pub hash: Vec<u8>,
}

/// BEP Response message.
#[derive(Debug, Clone)]
pub struct ResponseMessage {
    /// ID matching the originating Request.
    pub id: i32,
    /// Block payload bytes.
    pub data: Vec<u8>,
    /// Response code (0 = success).
    pub code: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_header_bep_roundtrip() {
        let header = MessageHeader {
            message_type: MessageType::Ping,
            message_id: 0,
            compressed: false,
        };

        let bep = header.to_bep_header();
        let decoded = MessageHeader::from_bep_header(&bep).unwrap();

        assert_eq!(decoded.message_type, MessageType::Ping);
        assert!(!decoded.compressed);
    }

    #[test]
    fn test_message_header_compression() {
        let header = MessageHeader {
            message_type: MessageType::Index,
            message_id: 0,
            compressed: true,
        };

        let bep = header.to_bep_header();
        assert_eq!(
            bep.compression,
            bep_protocol::messages::MessageCompression::Lz4 as i32
        );
        let decoded = MessageHeader::from_bep_header(&bep).unwrap();
        assert!(decoded.compressed);
    }

    #[test]
    fn test_hello_message_encode_decode() {
        let hello = HelloMessage {
            device_id: DeviceId::default(),
            device_name: "test-device".to_string(),
            client_name: "syncthing-rust".to_string(),
            client_version: "0.1.0".to_string(),
            protocols: vec!["bep/1.0".to_string()],
        };

        let encoded = hello.encode().unwrap();
        // Should start with magic (4 bytes) + length (2 bytes)
        assert!(encoded.len() >= 6);
        let magic = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
        assert_eq!(magic, bep_protocol::handshake::HELLO_MAGIC);

        let decoded = HelloMessage::decode(&encoded).unwrap();
        assert_eq!(decoded.device_name, "test-device");
        assert_eq!(decoded.client_name, "syncthing-rust");
        assert_eq!(decoded.client_version, "0.1.0");
    }
}
