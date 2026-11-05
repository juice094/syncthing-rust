//! Sync model traits and state definitions.

use crate::error::Result;
use crate::events::EventSubscriber;
use async_trait::async_trait;
use std::collections::HashMap;
use syncthing_core::types::{Config, Folder, FolderStatus};
use syncthing_core::DeviceId;

/// High-level sync operations exposed to upper layers (API, TUI, CLI).
#[async_trait]
pub trait SyncManager: Send + Sync {
    /// Returns the current configuration.
    async fn get_config(&self) -> Result<Config>;

    /// Updates the configuration.
    async fn update_config(&self, config: Config) -> Result<()>;

    /// Adds a device to the configuration.
    async fn add_device(&self, device: syncthing_core::types::Device) -> Result<()>;

    /// Removes a device from the configuration.
    async fn remove_device(&self, device_id: &DeviceId) -> Result<()>;

    /// Adds a folder to the configuration.
    async fn add_folder(&self, folder: Folder) -> Result<()>;

    /// Removes a folder from the configuration.
    async fn remove_folder(&self, folder_id: &str) -> Result<()>;

    /// Returns the current state of a folder.
    async fn get_folder_state(&self, folder_id: &str) -> Result<FolderState>;

    /// Starts the sync engine.
    async fn start(&self) -> Result<()>;

    /// Stops the sync engine.
    async fn stop(&self) -> Result<()>;

    /// Triggers a full scan of a folder.
    async fn scan_folder(&self, folder_id: &str) -> Result<()>;

    /// Triggers a scan of a subdirectory within a folder.
    async fn scan_folder_sub(&self, folder_id: &str, sub: &str) -> Result<()>;

    /// Triggers a pull round for a folder.
    async fn pull_folder(&self, folder_id: &str) -> Result<()>;

    /// Returns the list of currently connected devices.
    async fn get_connected_devices(&self) -> Result<Vec<DeviceId>>;

    /// Connects to a device.
    async fn connect_device(&self, device_id: DeviceId) -> Result<()>;

    /// Disconnects a device.
    async fn disconnect_device(&self, device_id: DeviceId) -> Result<()>;

    /// Returns a subscriber to the sync event stream.
    fn subscribe_events(&self) -> EventSubscriber;

    /// Returns aggregate sync statistics.
    async fn get_stats(&self) -> Result<SyncStats>;
}

/// Live state of a single folder.
#[derive(Debug, Clone)]
pub struct FolderState {
    /// Folder identifier.
    pub folder_id: String,
    /// Current folder status.
    pub status: FolderStatus,
    /// Time of the last completed scan, if any.
    pub last_scan: Option<chrono::DateTime<chrono::Utc>>,
    /// Time of the last completed pull, if any.
    pub last_pull: Option<chrono::DateTime<chrono::Utc>>,
    /// Number of files known locally.
    pub local_files: usize,
    /// Number of files known from remote indexes.
    pub remote_files: usize,
    /// Number of files currently in sync.
    pub in_sync_files: usize,
    /// Number of files that still need to be synced.
    pub need_files: usize,
    /// Recent errors for this folder.
    pub errors: Vec<String>,
}

impl FolderState {
    /// Creates an idle state for the given folder.
    pub fn new(folder_id: impl Into<String>) -> Self {
        Self {
            folder_id: folder_id.into(),
            status: FolderStatus::Idle,
            last_scan: None,
            last_pull: None,
            local_files: 0,
            remote_files: 0,
            in_sync_files: 0,
            need_files: 0,
            errors: Vec::new(),
        }
    }

    /// Returns the completion percentage (0-100).
    pub fn completion(&self) -> u8 {
        if self.local_files == 0 {
            return 100;
        }
        let synced = self.local_files.saturating_sub(self.need_files);
        ((synced as f64 / self.local_files as f64) * 100.0) as u8
    }
}

/// Aggregate sync statistics across all folders.
#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    /// Per-folder statistics.
    pub folders: HashMap<String, FolderStats>,
    /// Total number of files across all folders.
    pub total_files: usize,
    /// Total number of bytes across all folders.
    pub total_bytes: u64,
    /// Engine uptime in seconds.
    pub uptime_secs: u64,
}

/// File-count statistics for a single folder.
#[derive(Debug, Clone, Default)]
pub struct FolderStats {
    /// Number of regular files.
    pub files: usize,
    /// Number of directories.
    pub directories: usize,
    /// Number of symlinks.
    pub symlinks: usize,
    /// Total size in bytes.
    pub total_bytes: u64,
    /// Number of deleted entries.
    pub deleted: usize,
}

/// Handle to a running per-folder background task.
pub struct SyncTaskHandle {
    /// Folder the task belongs to.
    pub folder_id: String,
    /// Kind of background task.
    pub task_type: SyncTaskType,
    /// Handle used to abort the task.
    pub abort_handle: tokio::task::AbortHandle,
}

/// Kind of a per-folder background task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTaskType {
    Scan,
    Pull,
    Push,
}

/// A folder's configuration paired with its live state.
#[derive(Debug, Clone)]
pub struct FolderConfigState {
    /// Static folder configuration.
    pub config: Folder,
    /// Live folder state.
    pub state: FolderState,
    /// Last locally assigned sequence number.
    pub local_sequence: u64,
    /// Devices that share this folder.
    pub remote_devices: Vec<DeviceId>,
}

impl FolderConfigState {
    /// Creates a new entry from configuration, with fresh idle state.
    pub fn new(config: Folder) -> Self {
        let folder_id = config.id.clone();
        Self {
            config,
            state: FolderState::new(folder_id),
            local_sequence: 0,
            remote_devices: Vec::new(),
        }
    }

    /// Returns `true` when the folder is due for a scan.
    pub fn should_scan(&self) -> bool {
        if self.config.paused {
            return false;
        }

        matches!(
            self.state.status,
            FolderStatus::Idle | FolderStatus::SyncWaiting
        )
    }

    /// Returns `true` when the folder is due for a pull.
    pub fn should_pull(&self) -> bool {
        if self.config.paused {
            return false;
        }

        if !self.config.folder_type.can_sync() {
            return false;
        }

        matches!(
            self.state.status,
            FolderStatus::Idle | FolderStatus::ScanWaiting | FolderStatus::SyncWaiting
        )
    }
}
