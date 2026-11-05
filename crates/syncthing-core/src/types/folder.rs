//! Folder-related types.
//!
//! `FolderType` / `FolderStatus` / `Compression` / `Folder` / `FolderId` / `FolderSummary`.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::VersioningConfig;
use crate::DeviceId;

/// Folder type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderType {
    /// Send-receive (bidirectional sync).
    SendReceive,
    /// Send-only.
    SendOnly,
    /// Receive-only.
    ReceiveOnly,
    /// Receive-encrypted.
    ReceiveEncrypted,
}

impl FolderType {
    /// Returns whether local changes can be sent to peers.
    pub fn can_send(&self) -> bool {
        matches!(self, FolderType::SendReceive | FolderType::SendOnly)
    }

    /// Returns whether remote changes are applied locally.
    pub fn can_sync(&self) -> bool {
        matches!(
            self,
            FolderType::SendReceive | FolderType::ReceiveOnly | FolderType::ReceiveEncrypted
        )
    }
}

/// Folder status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderStatus {
    /// Idle.
    Idle,
    /// Waiting to scan.
    ScanWaiting,
    /// Scanning.
    Scanning,
    /// Waiting to sync.
    SyncWaiting,
    /// Syncing (pulling).
    Pulling,
    /// Syncing (pushing).
    Pushing,
    /// Sync complete.
    Synced,
    /// Paused.
    Paused,
    /// Error.
    Error,
}

/// Compression mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Compression {
    #[default]
    Metadata,
    Always,
    Never,
}

/// Folder configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    /// Folder ID.
    pub id: String,
    /// Folder path.
    pub path: String,
    /// Folder label (optional).
    pub label: Option<String>,
    /// Folder type.
    pub folder_type: FolderType,
    /// Whether the folder is paused.
    pub paused: bool,
    /// Rescan interval (seconds).
    pub rescan_interval_secs: i32,
    /// Devices sharing this folder.
    pub devices: Vec<DeviceId>,
    /// Ignore patterns.
    pub ignore_patterns: Vec<String>,
    /// Versioning configuration.
    pub versioning: Option<VersioningConfig>,
}

impl Folder {
    /// Creates a new folder configuration with default values.
    pub fn new(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            label: None,
            folder_type: FolderType::SendReceive,
            paused: false,
            rescan_interval_secs: 3600,
            devices: Vec::new(),
            ignore_patterns: Vec::new(),
            versioning: None,
        }
    }
}

/// Folder identifier.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId(String);

impl FolderId {
    /// Create from string
    pub fn new<S: Into<String>>(id: S) -> Self {
        Self(id.into())
    }

    /// Get as string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FolderId({})", self.0)
    }
}

impl fmt::Display for FolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Folder sync summary
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderSummary {
    /// Total files
    pub files: u64,
    /// Total directories
    pub directories: u64,
    /// Total symlinks
    pub symlinks: u64,
    /// Total bytes
    pub bytes: u64,
    /// Files needing sync
    pub need_files: u64,
    /// Directories needing sync
    pub need_directories: u64,
    /// Bytes needing sync
    pub need_bytes: u64,
    /// Pull errors
    pub pull_errors: u32,
}

impl FolderSummary {
    /// Check if folder is in sync
    pub fn is_synced(&self) -> bool {
        self.need_files == 0 && self.need_directories == 0 && self.need_bytes == 0
    }

    /// Calculate sync percentage
    pub fn sync_percent(&self) -> f64 {
        if self.bytes == 0 {
            return 100.0;
        }
        let synced = self.bytes - self.need_bytes;
        (synced as f64 / self.bytes as f64) * 100.0
    }
}
