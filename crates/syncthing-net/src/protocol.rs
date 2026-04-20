//! BEP 协议消息定义
//!
//! 参考: syncthing/lib/protocol/*.go

use bytes::Bytes;
use serde::Serialize;

use syncthing_core::DeviceId;
use bep_protocol::messages::Hello as BepHello;

/// BEP Magic 数字 (0x2EA7D90B)
pub const BEP_MAGIC: u32 = 0x2EA7D90B;

/// BEP Magic 字节
pub const BEP_MAGIC_BYTES: &[u8] = &BEP_MAGIC.to_be_bytes();

/// 消息类型
///
/// 与 Go 端 BEP MessageType 对齐：
/// ClusterConfig=0, Index=1, IndexUpdate=2, Request=3, Response=4,
/// DownloadProgress=5, Ping=6, Close=7
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MessageType {
    /// 集群配置
    ClusterConfig = 0,
    /// 索引消息
    Index = 1,
    /// 索引更新
    IndexUpdate = 2,
    /// 请求块
    Request = 3,
    /// 块响应
    Response = 4,
    /// 下载进度
    DownloadProgress = 5,
    /// Ping
    Ping = 6,
    /// 关闭连接
    Close = 7,
}

impl MessageType {
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

/// BEP 消息头
#[derive(Debug, Clone)]
pub struct MessageHeader {
    /// 消息类型
    pub message_type: MessageType,
    /// 消息ID（用于请求-响应匹配）
    pub message_id: u32,
    /// 压缩标志
    pub compressed: bool,
}

impl MessageHeader {
    /// 编码为 BEP Header protobuf
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
    
    /// 从 BEP Header protobuf 构造
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
            compressed: header.compression == bep_protocol::messages::MessageCompression::Lz4 as i32,
        })
    }
}

/// Hello 消息
#[derive(Debug, Clone)]
pub struct HelloMessage {
    /// 设备ID
    pub device_id: DeviceId,
    /// 设备名称
    pub device_name: String,
    /// 客户端名称
    pub client_name: String,
    /// 客户端版本
    pub client_version: String,
    /// 支持的协议
    pub protocols: Vec<String>,
}

impl HelloMessage {
    /// 创建新的Hello消息
    pub fn new(device_id: DeviceId) -> Self {
        Self {
            device_id,
            device_name: String::new(),
            client_name: "syncthing-rust".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            protocols: vec!["bep/1.0".to_string()],
        }
    }
    
    /// 编码为protobuf Hello字节
    /// 格式: [magic: u32 = 0x2EA7D90B][length: u16][protobuf Hello bytes]
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
    
    /// 从protobuf Hello字节解码
    /// 格式: [magic: u32 = 0x2EA7D90B][length: u16][protobuf Hello bytes]
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

/// 请求消息
#[derive(Debug, Clone, Serialize)]
pub struct RequestMessage {
    pub id: i32,
    pub folder: String,
    pub name: String,
    pub offset: i64,
    pub size: i32,
    pub hash: Vec<u8>,
}

/// 响应消息
#[derive(Debug, Clone)]
pub struct ResponseMessage {
    pub id: i32,
    pub data: Vec<u8>,
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
        assert_eq!(decoded.compressed, false);
    }

    #[test]
    fn test_message_header_compression() {
        let header = MessageHeader {
            message_type: MessageType::Index,
            message_id: 0,
            compressed: true,
        };

        let bep = header.to_bep_header();
        assert_eq!(bep.compression, bep_protocol::messages::MessageCompression::Lz4 as i32);
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
