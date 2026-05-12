//! BEP Protocol Messages
//!
//! 定义BEP协议的消息结构，使用Protobuf编码。
//! 所有 `prost::Message` 结构体的字段 tag 均与 Go 端 `internal/gen/bep/bep.pb.go`
//! 严格对齐（2026-04-11 验证通过，参见 VERIFICATION_REPORT_BEP_2026-04-11.md）。

use bytes::{BufMut, BytesMut};

/// Hello消息结构
///
/// 对应Go版本中的Hello消息，用于协议协商
/// Protobuf定义:
/// ```protobuf
/// message Hello {
///     string device_name = 1;
///     string client_name = 2;
///     string client_version = 3;
///     int32 num_connections = 4;
///     int64 timestamp = 5;
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Hello {
    /// 设备名称
    pub device_name: String,
    /// 客户端名称
    pub client_name: String,
    /// 客户端版本
    pub client_version: String,
    /// 连接数
    pub num_connections: i32,
    /// 时间戳（毫秒）
    pub timestamp: i64,
}

impl Hello {
    /// 创建新的Hello消息
    pub fn new(
        device_name: impl Into<String>,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self {
            device_name: device_name.into(),
            client_name: client_name.into(),
            client_version: client_version.into(),
            num_connections: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        }
    }

    /// 编码为Protobuf字节（零拷贝，返回 `Bytes`）
    ///
    /// TUNING_PLAN T-D1：避免 `encode_to_vec` 中的 `to_vec()` 拷贝。
    pub fn encode_to_bytes(&self) -> bytes::Bytes {
        let mut buf = BytesMut::new();

        // field 1: device_name (string, tag = 1 << 3 | 2 = 10)
        if !self.device_name.is_empty() {
            buf.put_u8(0x0a); // tag: field 1, wire type 2 (length-delimited)
            put_length_delimited(&mut buf, self.device_name.as_bytes());
        }

        // field 2: client_name (string, tag = 2 << 3 | 2 = 18)
        if !self.client_name.is_empty() {
            buf.put_u8(0x12); // tag: field 2, wire type 2
            put_length_delimited(&mut buf, self.client_name.as_bytes());
        }

        // field 3: client_version (string, tag = 3 << 3 | 2 = 26)
        if !self.client_version.is_empty() {
            buf.put_u8(0x1a); // tag: field 3, wire type 2
            put_length_delimited(&mut buf, self.client_version.as_bytes());
        }

        // field 4: num_connections (int32, tag = 4 << 3 | 0 = 32)
        if self.num_connections != 0 {
            buf.put_u8(0x20); // tag: field 4, wire type 0 (varint)
            put_varint(&mut buf, self.num_connections as u64);
        }

        // field 5: timestamp (int64, tag = 5 << 3 | 0 = 40)
        if self.timestamp != 0 {
            buf.put_u8(0x28); // tag: field 5, wire type 0
            put_varint(&mut buf, self.timestamp as u64);
        }

        buf.freeze()
    }

    /// 编码为Protobuf字节
    ///
    /// 手动实现Protobuf编码（简化版，不使用prost-build）
    pub fn encode_to_vec(&self) -> Vec<u8> {
        self.encode_to_bytes().to_vec()
    }

    /// 从Protobuf字节解码
    pub fn decode(buf: &[u8]) -> crate::Result<Self> {
        let mut hello = Hello {
            device_name: String::new(),
            client_name: String::new(),
            client_version: String::new(),
            num_connections: 0,
            timestamp: 0,
        };

        let mut pos = 0;
        while pos < buf.len() {
            if pos >= buf.len() {
                break;
            }

            let tag = buf[pos];
            pos += 1;

            let field_num = (tag >> 3) as i32;
            let wire_type = tag & 0x07;

            match (field_num, wire_type) {
                (1, 2) => {
                    // device_name
                    let (len, bytes_read) = read_varint(&buf[pos..])?;
                    pos += bytes_read;
                    if pos + len as usize > buf.len() {
                        return Err(crate::SyncthingError::protocol("truncated device_name"));
                    }
                    hello.device_name =
                        String::from_utf8_lossy(&buf[pos..pos + len as usize]).to_string();
                    pos += len as usize;
                }
                (2, 2) => {
                    // client_name
                    let (len, bytes_read) = read_varint(&buf[pos..])?;
                    pos += bytes_read;
                    if pos + len as usize > buf.len() {
                        return Err(crate::SyncthingError::protocol("truncated client_name"));
                    }
                    hello.client_name =
                        String::from_utf8_lossy(&buf[pos..pos + len as usize]).to_string();
                    pos += len as usize;
                }
                (3, 2) => {
                    // client_version
                    let (len, bytes_read) = read_varint(&buf[pos..])?;
                    pos += bytes_read;
                    if pos + len as usize > buf.len() {
                        return Err(crate::SyncthingError::protocol("truncated client_version"));
                    }
                    hello.client_version =
                        String::from_utf8_lossy(&buf[pos..pos + len as usize]).to_string();
                    pos += len as usize;
                }
                (4, 0) => {
                    // num_connections
                    let (val, bytes_read) = read_varint(&buf[pos..])?;
                    pos += bytes_read;
                    hello.num_connections = val as i32;
                }
                (5, 0) => {
                    // timestamp
                    let (val, bytes_read) = read_varint(&buf[pos..])?;
                    pos += bytes_read;
                    hello.timestamp = val as i64;
                }
                (_, 2) => {
                    // unknown string field, skip
                    let (len, bytes_read) = read_varint(&buf[pos..])?;
                    pos += bytes_read + len as usize;
                }
                (_, 0) => {
                    // unknown varint field, skip
                    let (_, bytes_read) = read_varint(&buf[pos..])?;
                    pos += bytes_read;
                }
                (_, 1) => {
                    // unknown 64-bit field, skip
                    pos += 8;
                }
                (_, 5) => {
                    // unknown 32-bit field, skip
                    pos += 4;
                }
                _ => {
                    return Err(crate::SyncthingError::protocol(format!(
                        "unknown wire type: {}",
                        wire_type
                    )));
                }
            }
        }

        Ok(hello)
    }
}

impl Default for Hello {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            client_name: "syncthing-rust".to_string(),
            client_version: "0.1.0".to_string(),
            num_connections: 1,
            timestamp: 0,
        }
    }
}

/// 写入变长整数
fn put_varint(buf: &mut BytesMut, mut value: u64) {
    while value >= 0x80 {
        buf.put_u8((value as u8) | 0x80);
        value >>= 7;
    }
    buf.put_u8(value as u8);
}

/// 读取变长整数
fn read_varint(buf: &[u8]) -> crate::Result<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    let mut pos = 0;

    loop {
        if pos >= buf.len() {
            return Err(crate::SyncthingError::protocol("truncated varint"));
        }

        let byte = buf[pos];
        pos += 1;

        result |= ((byte & 0x7f) as u64) << shift;
        if (byte & 0x80) == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(crate::SyncthingError::protocol("varint too large"));
        }
    }

    Ok((result, pos))
}

/// 写入长度分隔的字段
fn put_length_delimited(buf: &mut BytesMut, data: &[u8]) {
    put_varint(buf, data.len() as u64);
    buf.extend_from_slice(data);
}

// ============================================
// BEP message framing types
// ============================================

#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum MessageType {
    ClusterConfig = 0,
    Index = 1,
    IndexUpdate = 2,
    Request = 3,
    Response = 4,
    DownloadProgress = 5,
    Ping = 6,
    Close = 7,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum MessageCompression {
    None = 0,
    Lz4 = 1,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Header {
    #[prost(enumeration = "MessageType", tag = "1")]
    pub r#type: i32,
    #[prost(enumeration = "MessageCompression", tag = "2")]
    pub compression: i32,
}

// ============================================
// Prost-derived BEP wire types
// ============================================

#[derive(Clone, PartialEq, prost::Message)]
pub struct WireVector {
    #[prost(message, repeated, tag = "1")]
    pub counters: Vec<WireCounter>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct WireCounter {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(uint64, tag = "2")]
    pub value: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct WireBlockInfo {
    #[prost(int64, tag = "1")]
    pub offset: i64,
    #[prost(int32, tag = "2")]
    pub size: i32,
    #[prost(bytes, tag = "3")]
    pub hash: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum FileInfoType {
    File = 0,
    Directory = 1,
    SymlinkFile = 2,
    SymlinkDirectory = 3,
    Symlink = 4,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct WireFileInfo {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(enumeration = "FileInfoType", tag = "2")]
    pub r#type: i32,
    #[prost(int64, tag = "3")]
    pub size: i64,
    #[prost(uint32, tag = "4")]
    pub permissions: u32,
    #[prost(int64, tag = "5")]
    pub modified_s: i64,
    #[prost(bool, tag = "6")]
    pub deleted: bool,
    #[prost(bool, tag = "7")]
    pub invalid: bool,
    #[prost(bool, tag = "8")]
    pub no_permissions: bool,
    #[prost(message, optional, tag = "9")]
    pub version: Option<WireVector>,
    #[prost(int64, tag = "10")]
    pub sequence: i64,
    #[prost(int32, tag = "11")]
    pub modified_ns: i32,
    #[prost(uint64, tag = "12")]
    pub modified_by: u64,
    #[prost(int32, tag = "13")]
    pub block_size: i32,
    #[prost(message, optional, tag = "14")]
    pub platform: Option<PlatformData>,
    #[prost(message, repeated, tag = "16")]
    pub blocks: Vec<WireBlockInfo>,
    #[prost(bytes, tag = "17")]
    pub symlink_target: Vec<u8>,
    #[prost(bytes, tag = "18")]
    pub blocks_hash: Vec<u8>,
    #[prost(bytes, tag = "19")]
    pub encrypted: Vec<u8>,
    #[prost(bytes, tag = "20")]
    pub previous_blocks_hash: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PlatformData {
    #[prost(message, optional, tag = "1")]
    pub unix: Option<UnixData>,
    #[prost(message, optional, tag = "2")]
    pub windows: Option<WindowsData>,
    #[prost(message, optional, tag = "3")]
    pub linux: Option<XattrData>,
    #[prost(message, optional, tag = "4")]
    pub darwin: Option<XattrData>,
    #[prost(message, optional, tag = "5")]
    pub freebsd: Option<XattrData>,
    #[prost(message, optional, tag = "6")]
    pub netbsd: Option<XattrData>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct UnixData {
    #[prost(string, tag = "1")]
    pub owner_name: String,
    #[prost(string, tag = "2")]
    pub group_name: String,
    #[prost(int32, tag = "3")]
    pub uid: i32,
    #[prost(int32, tag = "4")]
    pub gid: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct WindowsData {
    #[prost(string, tag = "1")]
    pub owner_name: String,
    #[prost(bool, tag = "2")]
    pub owner_is_group: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct XattrData {
    #[prost(message, repeated, tag = "1")]
    pub xattrs: Vec<Xattr>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Xattr {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(bytes, tag = "2")]
    pub value: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Request {
    #[prost(int32, tag = "1")]
    pub id: i32,
    #[prost(string, tag = "2")]
    pub folder: String,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(int64, tag = "4")]
    pub offset: i64,
    #[prost(int32, tag = "5")]
    pub size: i32,
    #[prost(bytes, tag = "6")]
    pub hash: Vec<u8>,
    #[prost(bool, tag = "7")]
    pub from_temporary: bool,
    #[prost(int32, tag = "9")]
    pub block_no: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum ErrorCode {
    NoError = 0,
    Generic = 1,
    NoSuchFile = 2,
    InvalidFile = 3,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Response {
    #[prost(int32, tag = "1")]
    pub id: i32,
    #[prost(bytes, tag = "2")]
    pub data: Vec<u8>,
    #[prost(enumeration = "ErrorCode", tag = "3")]
    pub code: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Index {
    #[prost(string, tag = "1")]
    pub folder: String,
    #[prost(message, repeated, tag = "2")]
    pub files: Vec<WireFileInfo>,
    #[prost(int64, tag = "3")]
    pub last_sequence: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct IndexUpdate {
    #[prost(string, tag = "1")]
    pub folder: String,
    #[prost(message, repeated, tag = "2")]
    pub files: Vec<WireFileInfo>,
    #[prost(int64, tag = "3")]
    pub last_sequence: i64,
    #[prost(int64, tag = "4")]
    pub prev_sequence: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ClusterConfig {
    #[prost(message, repeated, tag = "1")]
    pub folders: Vec<WireFolder>,
    #[prost(bool, tag = "2")]
    pub secondary: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct WireDevice {
    #[prost(bytes, tag = "1")]
    pub id: Vec<u8>,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, repeated, tag = "3")]
    pub addresses: Vec<String>,
    #[prost(enumeration = "Compression", tag = "4")]
    pub compression: i32,
    #[prost(string, tag = "5")]
    pub cert_name: String,
    #[prost(int64, tag = "6")]
    pub max_sequence: i64,
    #[prost(bool, tag = "7")]
    pub introducer: bool,
    #[prost(uint64, tag = "8")]
    pub index_id: u64,
    #[prost(bool, tag = "9")]
    pub skip_introduction_removals: bool,
    #[prost(bytes, tag = "10")]
    pub encryption_password_token: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum Compression {
    Metadata = 0,
    Never = 1,
    Always = 2,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct WireFolder {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub label: String,
    #[prost(enumeration = "FolderType", tag = "3")]
    pub r#type: i32,
    #[prost(enumeration = "FolderStopReason", tag = "7")]
    pub stop_reason: i32,
    #[prost(message, repeated, tag = "16")]
    pub devices: Vec<WireDevice>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum FolderType {
    SendReceive = 0,
    SendOnly = 1,
    ReceiveOnly = 2,
    ReceiveEncrypted = 3,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum FolderStopReason {
    Running = 0,
    Paused = 1,
}

// ============================================
// Encode / decode helpers
// ============================================

pub fn encode_message<M: prost::Message>(msg: &M) -> crate::Result<bytes::Bytes> {
    let len = msg.encoded_len();
    let mut buf = bytes::BytesMut::with_capacity(len);
    msg.encode(&mut buf)
        .map_err(|e| crate::SyncthingError::protocol(format!("encode failed: {}", e)))?;
    Ok(buf.freeze())
}

pub fn decode_message<M: prost::Message + Default>(buf: &[u8]) -> crate::Result<M> {
    M::decode(buf).map_err(|e| crate::SyncthingError::protocol(format!("decode failed: {}", e)))
}

mod conversions;

#[cfg(test)]
mod tests;
