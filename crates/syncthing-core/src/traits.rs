//! Core trait definitions for Syncthing modules
//!
//! ⚠️ CRITICAL: Maintained by Master Agent
//! These traits define the contracts between modules. Worker Agents
//! must implement these traits exactly as specified.

use async_trait::async_trait;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::types::{BlockHash, FileInfo, FolderId};
use crate::DeviceId;

/// Transport type identifier for a connection path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportType {
    /// Plain TCP or TCP with TLS
    Tcp,
    /// QUIC
    Quic,
    /// Relay / DERP fallback
    Relay,
    /// In-memory pipe (for testing)
    Memory,
    /// Other / unknown
    Other,
}

/// Quality metrics for a network path.
#[derive(Debug, Clone)]
pub struct PathQuality {
    /// Smoothed round-trip time
    pub rtt: Duration,
    /// Packet loss ratio [0.0, 1.0]
    pub packet_loss: f64,
    /// Estimated bandwidth in bits per second, if known
    pub estimated_bps: Option<u64>,
    /// When this measurement was last updated
    pub last_updated: Instant,
}

impl Default for PathQuality {
    fn default() -> Self {
        Self {
            rtt: Duration::from_secs(1),
            packet_loss: 0.0,
            estimated_bps: None,
            last_updated: Instant::now(),
        }
    }
}

/// A generic reliable byte pipe abstracting TCP, QUIC, relay, or in-memory transports.
///
/// Implementors must provide [`tokio::io::AsyncRead`] and [`tokio::io::AsyncWrite`]
/// implementations so that BEP codec can operate on the pipe without knowing the
/// underlying transport.
pub trait ReliablePipe: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Sync + Unpin {
    /// Local endpoint address, if meaningful for the transport.
    fn local_addr(&self) -> Option<SocketAddr>;

    /// Peer endpoint address, if meaningful for the transport.
    fn peer_addr(&self) -> Option<SocketAddr>;

    /// Current quality estimate for this path.
    fn path_quality(&self) -> PathQuality {
        PathQuality::default()
    }

    /// Transport type discriminator.
    fn transport_type(&self) -> TransportType;
}

/// Type alias for boxed reliable pipes used by BEP connections.
pub type BoxedPipe = Box<dyn ReliablePipe>;

/// File system abstraction
///
/// Implementors: syncthing-fs crate (Worker-B)
#[async_trait]
pub trait FileSystem: Send + Sync {
    /// Read a block from a file at given offset
    ///
    /// # Arguments
    /// * `path` - Relative path within the folder
    /// * `offset` - Byte offset from start of file
    /// * `size` - Number of bytes to read
    ///
    /// # Returns
    /// The actual bytes read (may be less than size if at EOF)
    async fn read_block(&self, path: &Path, offset: u64, size: usize) -> Result<Vec<u8>>;

    /// Write a block to a file at given offset
    ///
    /// Creates the file if it doesn't exist.
    async fn write_block(&self, path: &Path, offset: u64, data: &[u8]) -> Result<()>;

    /// Calculate block hashes for a file
    ///
    /// Reads the file and computes SHA-256 hash for each block.
    async fn hash_file(&self, path: &Path) -> Result<Vec<BlockHash>>;

    /// Scan a directory recursively
    ///
    /// Returns FileInfo for all files, directories, and symlinks.
    async fn scan_directory(&self, path: &Path) -> Result<Vec<FileInfo>>;

    /// Get file info for a single file
    async fn file_info(&self, path: &Path) -> Result<FileInfo>;

    /// Remove a file or directory
    async fn remove(&self, path: &Path) -> Result<()>;

    /// Create a directory
    async fn create_dir(&self, path: &Path) -> Result<()>;

    /// Check if path exists
    async fn exists(&self, path: &Path) -> Result<bool>;

    /// Rename/move a file
    async fn rename(&self, from: &Path, to: &Path) -> Result<()>;
}

/// Type alias for boxed FileSystem
pub type FileSystemRef = Arc<dyn FileSystem>;

/// BEP Protocol connection
///
/// ⚠️ DEPRECATED: This trait is deprecated in favor of `ReliablePipe` + `BepSession`
/// in the `syncthing-net` crate. The old trait assumed a single-threaded, owned
/// connection model that does not match the current `Arc<BepConnection>` +
/// `BepSessionHandler` architecture.
#[deprecated(since = "0.1.0", note = "Use syncthing_net::BepSession with ReliablePipe instead")]
#[async_trait]
pub trait BepConnection: Send + Sync {
    /// Get remote device ID
    fn remote_device(&self) -> DeviceId;

    /// Send full index for a folder
    async fn send_index(&mut self, folder: &FolderId, files: Vec<FileInfo>) -> Result<()>;

    /// Send index update (delta)
    async fn send_index_update(&mut self, folder: &FolderId, files: Vec<FileInfo>) -> Result<()>;

    /// Request a block
    async fn request_block(
        &mut self,
        folder: &FolderId,
        hash: BlockHash,
        offset: u64,
        size: usize,
    ) -> Result<Vec<u8>>;

    /// Send a generic BEP message
    async fn send_message(&mut self, msg: &BepMessage) -> Result<()>;

    /// Receive next message
    ///
    /// Returns None if connection closed.
    async fn recv_message(&mut self) -> Result<Option<BepMessage>>;

    /// Close the connection gracefully
    async fn close(self) -> Result<()>;

    /// Check if connection is still alive
    fn is_alive(&self) -> bool;
}

/// Messages that can be received over BEP
///
/// ⚠️ DEPRECATED: Used only by the deprecated `BepConnection` trait.
#[deprecated(since = "0.1.0", note = "Use syncthing_net::BepSession with prost messages instead")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BepMessage {
    /// Full index
    Index {
        folder: FolderId,
        files: Vec<FileInfo>,
    },
    /// Index update (delta)
    IndexUpdate {
        folder: FolderId,
        files: Vec<FileInfo>,
    },
    /// Block request
    Request {
        /// Request ID used to match with the corresponding response
        id: u64,
        folder: FolderId,
        hash: BlockHash,
        offset: u64,
        size: usize,
    },
    /// Block response with data
    Response {
        /// Request ID from the original request
        id: u64,
        hash: BlockHash,
        data: Vec<u8>,
    },
    /// Download progress update
    DownloadProgress {
        folder: FolderId,
        file: String,
        total: u64,
        done: u64,
    },
    /// Ping (keepalive)
    Ping,
    /// Pong (keepalive response)
    Pong,
    /// Cluster configuration
    ClusterConfig {
        /// Folder IDs in the cluster config
        folders: Vec<String>,
        /// Whether this is a secondary connection
        secondary: bool,
    },
}

/// Block storage
///
/// Implementors: syncthing-db crate (Worker-E)
#[async_trait]
pub trait BlockStore: Send + Sync {
    /// Store a block
    async fn put(&self, hash: BlockHash, data: &[u8]) -> Result<()>;

    /// Retrieve a block
    ///
    /// Returns None if block not found.
    async fn get(&self, hash: BlockHash) -> Result<Option<Vec<u8>>>;

    /// Check if block exists
    async fn has(&self, hash: BlockHash) -> Result<bool>;

    /// Delete a block
    async fn delete(&self, hash: BlockHash) -> Result<()>;

    /// Get index for a folder
    async fn get_index(&self, folder: &FolderId) -> Result<Vec<FileInfo>>;

    /// Update index for a folder
    ///
    /// Replaces the entire index. Use update_index_delta for partial updates.
    async fn update_index(&self, folder: &FolderId, files: Vec<FileInfo>) -> Result<()>;

    /// Update index with delta
    async fn update_index_delta(
        &self,
        folder: &FolderId,
        files: Vec<FileInfo>,
    ) -> Result<()>;

    /// Get folder statistics
    async fn folder_stats(&self, folder: &FolderId) -> Result<FolderStats>;
}

/// Folder statistics
#[derive(Debug, Clone, Default)]
pub struct FolderStats {
    /// Number of files
    pub file_count: u64,
    /// Total bytes
    pub total_bytes: u64,
    /// Number of blocks stored
    pub block_count: u64,
}

/// Type alias for boxed BlockStore
pub type BlockStoreRef = Arc<dyn BlockStore>;

/// Device discovery
///
/// Implementors: syncthing-net crate (Worker-D)
#[async_trait]
pub trait Discovery: Send + Sync {
    /// Look up addresses for a device
    ///
    /// Returns list of addresses in order of preference.
    async fn lookup(&self, device: &DeviceId) -> Result<Vec<String>>;

    /// Announce this device to the discovery system
    async fn announce(&self, device: &DeviceId, addresses: Vec<String>) -> Result<()>;

    /// Start periodic announcement
    async fn start_periodic_announce(
        &self,
        device: DeviceId,
        addresses: Vec<String>,
        interval_secs: u64,
    ) -> Result<Box<dyn AnnouncementHandle>>;
}

/// Handle for controlling periodic announcement
#[async_trait]
pub trait AnnouncementHandle: Send + Sync {
    /// Stop the announcement
    async fn stop(self) -> Result<()>;
}

/// No-op discovery implementation for when discovery is disabled
pub struct NoopDiscovery;

#[async_trait]
impl Discovery for NoopDiscovery {
    async fn lookup(&self, _device: &DeviceId) -> Result<Vec<String>> {
        Ok(vec![])
    }

    async fn announce(&self, _device: &DeviceId, _addresses: Vec<String>) -> Result<()> {
        Ok(())
    }

    async fn start_periodic_announce(
        &self,
        _device: DeviceId,
        _addresses: Vec<String>,
        _interval_secs: u64,
    ) -> Result<Box<dyn AnnouncementHandle>> {
        Ok(Box::new(NoopHandle))
    }
}

struct NoopHandle;

#[async_trait]
impl AnnouncementHandle for NoopHandle {
    async fn stop(self) -> Result<()> {
        Ok(())
    }
}

/// Transport layer for connections
///
/// Implementors: syncthing-net crate (Worker-D)
#[async_trait]
pub trait Transport: Send + Sync {
    /// Connect to a device at given address
    ///
    /// Address format depends on transport:
    /// - TCP: "tcp://host:port"
    /// - QUIC: "quic://host:port"
    /// - Relay: "relay://host:port?id=..."
    async fn connect(&self, addr: &str, expected_device: Option<DeviceId>) -> Result<Box<dyn BepConnection>>;

    /// Listen for incoming connections
    async fn listen(&self, bind_addr: &str) -> Result<Box<dyn ConnectionListener>>;
}

/// Connection listener
#[async_trait]
pub trait ConnectionListener: Send + Sync {
    /// Accept next incoming connection
    ///
    /// Returns None if listener closed.
    async fn accept(&mut self) -> Result<Option<Box<dyn BepConnection>>>;

    /// Close the listener
    async fn close(self) -> Result<()>;

    /// Get local address
    fn local_addr(&self) -> Result<String>;
}

/// Event publisher for internal events
///
/// Implementors: syncthing-api crate (Worker-F)
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish an event
    async fn publish(&self, event: crate::types::Event) -> Result<()>;

    /// Subscribe to events
    async fn subscribe(&self, filter: EventFilter) -> Result<Box<dyn EventStream>>;
}

/// Event subscription filter
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Filter by event types (empty = all types)
    pub types: Vec<String>,
    /// Filter by folder (None = all folders)
    pub folder: Option<FolderId>,
    /// Filter by device (None = all devices)
    pub device: Option<DeviceId>,
}

/// Event stream for receiving events
#[async_trait]
pub trait EventStream: Send + Sync {
    /// Receive next event
    ///
    /// Returns None if stream closed.
    async fn recv(&mut self) -> Result<Option<crate::types::Event>>;

    /// Close the stream
    async fn close(self) -> Result<()>;
}

/// Configuration storage
///
/// Implementors: syncthing-api crate (Worker-F)
#[async_trait]
pub trait ConfigStore: Send + Sync {
    /// Load configuration
    async fn load(&self) -> Result<Config>;

    /// Save configuration
    async fn save(&self, config: &Config) -> Result<()>;

    /// Watch for configuration changes
    async fn watch(&self) -> Result<Box<dyn ConfigStream>>;
}

/// Config change stream
#[async_trait]
pub trait ConfigStream: Send + Sync {
    /// Wait for next config change
    async fn next(&mut self) -> Result<()>;
}

/// Synchronization model interface
///
/// Implementors: syncthing-sync crate (Worker-C)
#[async_trait]
pub trait SyncModel: Send + Sync {
    /// Start synchronizing a folder
    async fn start_folder(&self, folder: FolderId) -> Result<()>;

    /// Stop synchronizing a folder
    async fn stop_folder(&self, folder: FolderId) -> Result<()>;

    /// Scan a folder for local changes and update index
    async fn scan_folder(&self, folder: &FolderId) -> Result<()>;

    /// Request pull (download) for a folder
    async fn pull(&self, folder: &FolderId) -> Result<SyncResult>;

    /// Get sync status for a folder
    async fn folder_status(&self, folder: &FolderId) -> Result<FolderStatus>;

}

/// Result of a sync operation
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Files processed
    pub files_processed: u32,
    /// Bytes transferred
    pub bytes_transferred: u64,
    /// Errors encountered
    pub errors: Vec<String>,
}

/// Folder sync status
#[derive(Debug, Clone)]
pub enum FolderStatus {
    /// Idle, in sync
    Idle,
    /// Scanning local files
    Scanning,
    /// Synchronizing (pulling)
    Syncing { progress: f64 },
    /// Error state
    Error { message: String },
    /// Paused
    Paused,
}

pub use crate::types::Config;
