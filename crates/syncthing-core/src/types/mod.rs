//! Core type definitions.

use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;

mod connection;
pub use connection::*;

mod folder;
pub use folder::*;

/// File type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
}

/// Block information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlockInfo {
    /// Block size.
    pub size: i32,
    /// Block hash (SHA-256).
    pub hash: Vec<u8>,
    /// Offset within the file.
    pub offset: i64,
}

/// Version vector, used for conflict detection and resolution.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vector {
    /// Maps a device counter ID to its counter value.
    pub counters: HashMap<u64, u64>,
}

impl Vector {
    /// Creates a new empty version vector.
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }

    /// Adds or updates the counter for a device.
    pub fn with_counter(mut self, device_id: u64, counter: u64) -> Self {
        self.counters.insert(device_id, counter);
        self
    }

    /// Increments the counter for the given device.
    pub fn increment(&mut self, device_id: u64) {
        *self.counters.entry(device_id).or_insert(0) += 1;
    }

    /// Returns the counter value for the given device (0 if absent).
    pub fn get(&self, device_id: u64) -> u64 {
        self.counters.get(&device_id).copied().unwrap_or(0)
    }

    /// Compares two version vectors.
    pub fn compare(&self, other: &Vector) -> VersionComparison {
        let mut has_greater = false;
        let mut has_less = false;

        let all_devices: std::collections::HashSet<_> =
            self.counters.keys().chain(other.counters.keys()).collect();

        for device in all_devices {
            let self_count = self.get(*device);
            let other_count = other.get(*device);

            if self_count > other_count {
                has_greater = true;
            } else if self_count < other_count {
                has_less = true;
            }
        }

        match (has_greater, has_less) {
            (true, true) => VersionComparison::Conflict,
            (true, false) => VersionComparison::Greater,
            (false, true) => VersionComparison::Less,
            (false, false) => VersionComparison::Equal,
        }
    }

    /// Returns whether this version dominates `other` (every counter is
    /// greater than or equal to the other's).
    pub fn dominates(&self, other: &Vector) -> bool {
        let all_devices: std::collections::HashSet<_> =
            self.counters.keys().chain(other.counters.keys()).collect();

        for device in all_devices {
            let self_count = self.get(*device);
            let other_count = other.get(*device);

            if self_count < other_count {
                return false;
            }
        }

        true
    }
}

/// Result of comparing two version vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionComparison {
    /// The vectors are equal.
    Equal,
    /// This vector is greater (dominates without conflict).
    Greater,
    /// This vector is less (is dominated).
    Less,
    /// The vectors are concurrent (both have higher counters somewhere).
    Conflict,
}

/// Concurrent direction, used as the conflict-resolution tiebreak (aligned
/// with Go `Vector.Compare`'s ConcurrentGreater / ConcurrentLesser
/// semantics: the direction of the first diverging counter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrentOrder {
    /// The first diverging counter is greater on this side (Go ConcurrentGreater).
    SelfGreater,
    /// The first diverging counter is greater on the other side (Go ConcurrentLesser).
    OtherGreater,
}

impl Vector {
    /// Returns the directed concurrent ordering (aligned with Go
    /// `lib/protocol/vector.go` `Compare`).
    ///
    /// Returns `Some(direction)` only when the two vectors are truly
    /// concurrent (each side has a higher counter somewhere); the direction
    /// is the relation of the first diverging counter, and a direction flip
    /// between counters signals concurrency. Returns `None` for dominance or
    /// equality. The direction is deterministic and identical on both ends,
    /// which makes it usable as the conflict-resolution tiebreak.
    pub fn concurrent_order(&self, other: &Vector) -> Option<ConcurrentOrder> {
        let mut ids: std::collections::BTreeSet<u64> = self.counters.keys().copied().collect();
        ids.extend(other.counters.keys().copied());
        let mut first: Option<ConcurrentOrder> = None;
        for id in ids {
            let a = self.get(id);
            let b = other.get(id);
            let dir = if a > b {
                ConcurrentOrder::SelfGreater
            } else if a < b {
                ConcurrentOrder::OtherGreater
            } else {
                continue;
            };
            match first {
                None => first = Some(dir),
                Some(d) if d != dir => return first,
                _ => {}
            }
        }
        None
    }
}

/// Index ID (8 random bytes).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct IndexID(pub [u8; 8]);

impl IndexID {
    /// Generates a new random index ID.
    pub fn random() -> Self {
        let mut bytes = [0u8; 8];
        rand::rng().fill(&mut bytes);
        Self(bytes)
    }

    /// Creates an index ID from a u64.
    pub fn from_u64(value: u64) -> Self {
        Self(value.to_be_bytes())
    }

    /// Converts the index ID to a u64.
    pub fn as_u64(&self) -> u64 {
        u64::from_be_bytes(self.0)
    }
}

impl fmt::Debug for IndexID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IndexID({:016x})", self.as_u64())
    }
}

/// Index delta.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexDelta {
    /// Folder ID.
    pub folder: String,
    /// Index ID.
    pub index_id: IndexID,
    /// Starting sequence number.
    pub start_sequence: u64,
    /// File list.
    pub files: Vec<FileInfo>,
}

/// Base version of a file at last global consensus, used for three-way merge.
/// Full blocks are not stored, to avoid DB bloat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileInfoBase {
    /// File size.
    pub size: i64,
    /// Modification time (seconds).
    pub modified_s: i64,
    /// Modification time (nanoseconds part).
    pub modified_ns: i32,
    /// Block-list hash (if the original FileInfo had one).
    pub blocks_hash: Option<Vec<u8>>,
    /// Whole-content hash (fallback identity).
    pub content_hash: Option<Vec<u8>>,
}

/// File information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileInfo {
    /// File name (relative path).
    pub name: String,
    /// File type.
    pub file_type: FileType,
    /// File size.
    pub size: i64,
    /// File permissions (Unix mode).
    pub permissions: u32,
    /// Modification time (seconds).
    pub modified_s: i64,
    /// Modification time (nanoseconds part).
    pub modified_ns: i32,
    /// Version vector.
    pub version: Vector,
    /// Sequence number (for index ordering).
    pub sequence: u64,
    /// Block size.
    pub block_size: i32,
    /// Block list.
    pub blocks: Vec<BlockInfo>,
    /// Symlink target (if a symlink).
    pub symlink_target: Option<String>,
    /// Deletion marker.
    pub deleted: Option<bool>,
    /// Short ID of the device that last modified the file (BEP compatibility field).
    pub modified_by: Option<u64>,
    /// Block-list hash for quick change comparison.
    pub blocks_hash: Option<Vec<u8>>,
    /// No-permissions flag (BEP compatibility field).
    pub no_permissions: Option<bool>,
    /// Last globally consistent base version (used for three-way merge).
    #[serde(default)]
    pub base_version: Option<FileInfoBase>,
}

impl FileInfo {
    /// Creates a new FileInfo with only the name set; everything else defaults.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            file_type: FileType::File,
            size: 0,
            permissions: 0,
            modified_s: 0,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 0,
            block_size: 0,
            blocks: Vec::new(),
            symlink_target: None,
            deleted: Some(false),
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        }
    }

    /// Returns whether the file is deleted.
    pub fn is_deleted(&self) -> bool {
        self.deleted.unwrap_or(false)
    }

    /// Marks the file as deleted.
    pub fn mark_deleted(&mut self) {
        self.deleted = Some(true);
    }
}

/// Device configuration (API view).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Device ID.
    pub id: String,
    /// Device name.
    pub name: String,
    /// Device addresses.
    pub addresses: Vec<String>,
    /// Whether the device is paused.
    pub paused: bool,
    /// Whether the device is an introducer.
    pub introducer: bool,
}

/// Folder configuration (API view).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderConfig {
    /// Folder ID.
    pub id: String,
    /// Folder label.
    pub label: String,
    /// Folder path.
    pub path: String,
    /// Devices sharing this folder.
    pub devices: Vec<String>,
    /// Rescan interval (seconds).
    pub rescan_interval_secs: u32,
    /// Versioning configuration.
    pub versioning: VersioningConfig,
}

/// GUI (REST API) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiConfig {
    /// Whether the GUI is enabled.
    pub enabled: bool,
    /// GUI listen address.
    pub address: String,
    /// Admin API key (full access).
    pub api_key: String,
    /// Read-only API key (GET/HEAD only; POST/PUT/DELETE are rejected).
    #[serde(default)]
    pub ro_api_key: String,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            address: crate::constants::DEFAULT_GUI_ADDR.to_string(),
            api_key: String::new(),
            ro_api_key: String::new(),
        }
    }
}

/// Global options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Options {
    /// Listen addresses.
    pub listen_addresses: Vec<String>,
    /// Whether global announcement is enabled.
    pub global_announce_enabled: bool,
    /// Whether local announcement is enabled.
    pub local_announce_enabled: bool,
    /// Whether relays are enabled.
    pub relays_enabled: bool,
    /// Enabled transports, e.g. ["tcp", "websocket", "relay"].
    #[serde(default = "default_transports")]
    pub transports: Vec<String>,
}

/// Versioning configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[derive(Default)]
pub enum VersioningConfig {
    /// Versioning disabled.
    #[default]
    None,
    /// Simple versioning.
    Simple {
        /// Strategy parameters.
        params: HashMap<String, String>,
    },
    /// Staggered versioning.
    Staggered {
        /// Strategy parameters.
        params: HashMap<String, String>,
    },
    /// External versioning command.
    External {
        /// Strategy parameters.
        params: HashMap<String, String>,
    },
}

/// Device configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Device ID.
    pub id: crate::DeviceId,
    /// Device name.
    pub name: Option<String>,
    /// Device addresses.
    pub addresses: Vec<AddressType>,
    /// Whether the device is paused.
    pub paused: bool,
    /// Whether the device is an introducer.
    pub introducer: bool,
}

/// Top-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Config version.
    pub version: i32,
    /// Listen address.
    #[serde(default = "default_listen")]
    pub listen_addr: String,
    /// Device name.
    #[serde(default = "default_device_name")]
    pub device_name: String,
    /// Folders.
    pub folders: Vec<Folder>,
    /// Devices.
    pub devices: Vec<Device>,
    /// Local device ID.
    pub local_device_id: Option<crate::DeviceId>,
    /// GUI configuration.
    #[serde(default)]
    pub gui: GuiConfig,
    /// Options.
    #[serde(default)]
    pub options: Options,
}

fn default_listen() -> String {
    crate::constants::DEFAULT_LISTEN_ADDR.to_string()
}
fn default_transports() -> Vec<String> {
    vec!["tcp".to_string()]
}
fn default_device_name() -> String {
    "syncthing-rust".to_string()
}

impl Config {
    /// Creates a new empty configuration.
    pub fn new() -> Self {
        Self {
            version: 1,
            listen_addr: default_listen(),
            device_name: default_device_name(),
            folders: Vec::new(),
            devices: Vec::new(),
            local_device_id: None,
            gui: GuiConfig::default(),
            options: Options::default(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// Full index message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    /// Folder ID.
    pub folder: String,
    /// File list.
    pub files: Vec<FileInfo>,
}

/// Index update message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexUpdate {
    /// Folder ID.
    pub folder: String,
    /// Updated file list.
    pub files: Vec<FileInfo>,
}

/// Block hash (SHA-256)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockHash([u8; 32]);

impl BlockHash {
    /// Create from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Get raw bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Calculate hash from data
    pub fn from_data(data: &[u8]) -> Self {
        let hash = Sha256::digest(data);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hash);
        Self(bytes)
    }

    /// Convert to Vec\<u8\>
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl fmt::Debug for BlockHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlockHash({})", hex::encode(&self.0[..8]))
    }
}

impl fmt::Display for BlockHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// Event types for the event system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    /// Folder state changed
    FolderSummary {
        /// Folder ID
        folder: FolderId,
        /// Summary data
        summary: FolderSummary,
    },
    /// File was downloaded
    ItemFinished {
        /// Folder ID
        folder: FolderId,
        /// Item name
        item: String,
        /// Error message if any
        error: Option<String>,
    },
    /// Device connected
    DeviceConnected {
        /// Device ID
        device: crate::DeviceId,
        /// Connection address
        addr: String,
    },
    /// Device disconnected
    DeviceDisconnected {
        /// Device ID
        device: crate::DeviceId,
        /// Error message if any
        error: Option<String>,
    },
    /// Local index updated
    LocalIndexUpdated {
        /// Folder ID
        folder: FolderId,
        /// Updated items
        items: Vec<String>,
    },
    /// Remote index received
    RemoteIndexUpdated {
        /// Device ID
        device: crate::DeviceId,
        /// Folder ID
        folder: FolderId,
        /// Number of items
        items_count: usize,
    },
}

#[cfg(test)]
mod tests;
