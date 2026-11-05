//! BEP protocol messages.
//!
//! Protobuf-encoded BEP message structures. Field tags of every
//! `prost::Message` struct match Go's `internal/gen/bep/bep.pb.go` exactly
//! (verified 2026-04-11, see VERIFICATION_REPORT_BEP_2026-04-11.md).

pub use prost::Message;

/// BEP v1 Hello message.
///
/// Corresponds to the Go Hello message; field tags match Go
/// `internal/gen/bep/bep.pb.go` exactly.
///
/// Protobuf definition:
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
    /// Creates a Hello message with the given names, the current timestamp,
    /// and `num_connections = 1`.
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

    /// Default Hello for tests and placeholder use.
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

/// BEP message type discriminator (`Header.type`).
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

/// Compression flag for a BEP message.
#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum MessageCompression {
    None = 0,
    Lz4 = 1,
}

/// BEP message header.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Header {
    #[prost(enumeration = "MessageType", tag = "1")]
    pub r#type: i32,
    #[prost(enumeration = "MessageCompression", tag = "2")]
    pub compression: i32,
}

/// Wire form of a version vector.
#[derive(Clone, PartialEq, prost::Message)]
pub struct WireVector {
    #[prost(message, repeated, tag = "1")]
    pub counters: Vec<WireCounter>,
}

/// One counter entry of a wire version vector.
#[derive(Clone, PartialEq, prost::Message)]
pub struct WireCounter {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(uint64, tag = "2")]
    pub value: u64,
}

/// Wire form of a block entry.
#[derive(Clone, PartialEq, prost::Message)]
pub struct WireBlockInfo {
    #[prost(int64, tag = "1")]
    pub offset: i64,
    #[prost(int32, tag = "2")]
    pub size: i32,
    #[prost(bytes, tag = "3")]
    pub hash: Vec<u8>,
}

/// Wire file-type enumeration (matches Go FileInfoType).
#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum FileInfoType {
    File = 0,
    Directory = 1,
    SymlinkFile = 2,
    SymlinkDirectory = 3,
    Symlink = 4,
}

/// Wire form of FileInfo.
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

/// Per-platform metadata container.
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

/// Unix ownership metadata.
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

/// Windows ownership metadata.
#[derive(Clone, PartialEq, prost::Message)]
pub struct WindowsData {
    #[prost(string, tag = "1")]
    pub owner_name: String,
    #[prost(bool, tag = "2")]
    pub owner_is_group: bool,
}

/// Extended-attribute container.
#[derive(Clone, PartialEq, prost::Message)]
pub struct XattrData {
    #[prost(message, repeated, tag = "1")]
    pub xattrs: Vec<Xattr>,
}

/// One extended attribute (name/value pair).
#[derive(Clone, PartialEq, prost::Message)]
pub struct Xattr {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(bytes, tag = "2")]
    pub value: Vec<u8>,
}

/// Block request message.
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

/// BEP response error codes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum ErrorCode {
    NoError = 0,
    Generic = 1,
    NoSuchFile = 2,
    InvalidFile = 3,
}

/// Block response message.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Response {
    #[prost(int32, tag = "1")]
    pub id: i32,
    #[prost(bytes, tag = "2")]
    pub data: Vec<u8>,
    #[prost(enumeration = "ErrorCode", tag = "3")]
    pub code: i32,
}

/// Full index message (wire form).
#[derive(Clone, PartialEq, prost::Message)]
pub struct Index {
    #[prost(string, tag = "1")]
    pub folder: String,
    #[prost(message, repeated, tag = "2")]
    pub files: Vec<WireFileInfo>,
    #[prost(int64, tag = "3")]
    pub last_sequence: i64,
}

/// Index delta message (wire form).
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

/// ClusterConfig message.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ClusterConfig {
    #[prost(message, repeated, tag = "1")]
    pub folders: Vec<WireFolder>,
    #[prost(bool, tag = "2")]
    pub secondary: bool,
}

/// Device entry inside ClusterConfig.
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

/// Compression negotiation values.
#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum Compression {
    Metadata = 0,
    Never = 1,
    Always = 2,
}

/// Folder entry inside ClusterConfig.
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

/// Folder type enumeration (wire form).
#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum FolderType {
    SendReceive = 0,
    SendOnly = 1,
    ReceiveOnly = 2,
    ReceiveEncrypted = 3,
}

/// Folder stop-reason enumeration (wire form).
#[derive(Clone, Copy, PartialEq, Eq, Debug, prost::Enumeration)]
pub enum FolderStopReason {
    Running = 0,
    Paused = 1,
}

/// Encodes a prost message to bytes.
///
/// # Errors
/// Returns a protocol error if encoding fails.
pub fn encode_message<M: prost::Message>(msg: &M) -> crate::Result<bytes::Bytes> {
    let len = msg.encoded_len();
    let mut buf = bytes::BytesMut::with_capacity(len);
    msg.encode(&mut buf)
        .map_err(|e| crate::SyncthingError::protocol(format!("encode failed: {}", e)))?;
    Ok(buf.freeze())
}

/// Decodes a prost message from bytes.
///
/// # Errors
/// Returns a protocol error if decoding fails.
pub fn decode_message<M: prost::Message + Default>(buf: &[u8]) -> crate::Result<M> {
    M::decode(buf).map_err(|e| crate::SyncthingError::protocol(format!("decode failed: {}", e)))
}

mod conversions;

#[cfg(test)]
mod tests;
