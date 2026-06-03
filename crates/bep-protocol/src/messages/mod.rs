//! BEP Protocol Messages
//!
//! 定义BEP协议的消息结构，使用Protobuf编码。
//! 所有 `prost::Message` 结构体的字段 tag 均与 Go 端 `internal/gen/bep/bep.pb.go`
//! 严格对齐（2026-04-11 验证通过，参见 VERIFICATION_REPORT_BEP_2026-04-11.md）。

pub use prost::Message;

/// Hello消息结构
///
/// 对应Go版本中的Hello消息（BEP v1），字段 tag 与 Go `internal/gen/bep/bep.pb.go` 严格对齐。
///
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
#[derive(Clone, PartialEq, prost::Message)]
pub struct Hello {
    #[prost(string, tag = "1")]
    pub device_name: String,
    #[prost(string, tag = "2")]
    pub client_name: String,
    #[prost(string, tag = "3")]
    pub client_version: String,
    #[prost(int32, tag = "4")]
    pub num_connections: i32,
    #[prost(int64, tag = "5")]
    pub timestamp: i64,
}

impl Hello {
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

    /// Default Hello for tests and placeholder use
    pub fn default_rust_client() -> Self {
        Self {
            device_name: String::new(),
            client_name: "syncthing-rust".to_string(),
            client_version: "0.1.0".to_string(),
            num_connections: 1,
            timestamp: 0,
        }
    }
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
