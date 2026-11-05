//! Sync event system.
//!
//! Provides a publish/subscribe mechanism for state-change notifications.

use syncthing_core::types::{FileInfo, Folder, Vector};
use tokio::sync::broadcast;

/// Events emitted by the sync engine.
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// Local index has been updated.
    LocalIndexUpdated {
        folder: String,
        files: Vec<FileInfo>,
    },

    /// Index update received from a remote device.
    RemoteIndexReceived {
        folder: String,
        device: syncthing_core::DeviceId,
        files: Vec<FileInfo>,
    },

    /// A folder scan completed.
    FolderScanCompleted {
        folder: String,
        files_changed: usize,
    },

    /// A folder scan failed.
    FolderScanFailed { folder: String, error: String },

    /// Size of the watcher's dirty-path set (indicates incremental scan pressure).
    WatcherDirtySetSize { folder: String, size: usize },

    /// Number of watcher events dropped due to channel backpressure.
    WatcherEventsDropped { folder: String, dropped: u64 },

    /// An incremental (watcher-triggered) scan was triggered, as opposed to a full scan.
    IncrementalScanTriggered { folder: String, paths: usize },

    /// Processing of an item has started.
    ItemStarted {
        folder: String,
        item: String,
        action: ItemAction,
    },

    /// Processing of an item has finished.
    ItemFinished {
        folder: String,
        item: String,
        action: ItemAction,
        error: Option<String>,
    },

    /// A conflict between local and remote versions was detected.
    ConflictDetected {
        folder: String,
        item: String,
        local_version: Vector,
        remote_version: Vector,
    },

    /// A previously detected conflict has been resolved.
    ConflictResolved {
        folder: String,
        item: String,
        resolution: ConflictResolution,
    },

    /// Progress of a file download.
    DownloadProgress {
        folder: String,
        file: String,
        bytes_done: u64,
        bytes_total: u64,
    },

    /// Progress of a file upload.
    UploadProgress {
        folder: String,
        file: String,
        bytes_done: u64,
        bytes_total: u64,
    },

    /// A folder's state has changed.
    FolderStateChanged {
        folder: String,
        from: syncthing_core::types::FolderStatus,
        to: syncthing_core::types::FolderStatus,
    },

    /// Folder configuration was updated.
    FolderConfigUpdated { folder: Folder },

    /// A device has connected.
    DeviceConnected { device: syncthing_core::DeviceId },

    /// A device has disconnected.
    DeviceDisconnected {
        device: syncthing_core::DeviceId,
        reason: String,
    },

    /// Synchronization of a folder completed.
    SyncComplete { folder: String, stats: SyncStats },

    /// A chunk of an IndexUpdate message has been sent.
    IndexUpdateChunkSent {
        folder: String,
        device: syncthing_core::DeviceId,
        chunk_index: usize,
        total_chunks: usize,
        files: usize,
        bytes: usize,
    },
}

/// Action performed on an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemAction {
    Add,
    Modify,
    Delete,
    Conflict,
}

/// How a conflict was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    UseLocal,
    UseRemote,
    Merge,
    RenameBoth,
}

/// Statistics for a completed sync round.
#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    pub files_added: usize,
    pub files_modified: usize,
    pub files_deleted: usize,
    pub bytes_transferred: u64,
    pub conflicts_detected: usize,
    pub conflicts_resolved: usize,
}

/// Broadcast publisher for [`SyncEvent`]s.
#[derive(Debug, Clone)]
pub struct EventPublisher {
    sender: broadcast::Sender<SyncEvent>,
}

impl EventPublisher {
    /// Creates a new publisher with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publishes an event; slow subscribers that lag behind are silently dropped.
    pub fn publish(&self, event: SyncEvent) {
        let _ = self.sender.send(event);
    }

    /// Returns a new subscriber.
    pub fn subscribe(&self) -> EventSubscriber {
        EventSubscriber {
            receiver: self.sender.subscribe(),
        }
    }
}

impl Default for EventPublisher {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// Receiving end of the sync event broadcast channel.
#[derive(Debug)]
pub struct EventSubscriber {
    receiver: broadcast::Receiver<SyncEvent>,
}

impl EventSubscriber {
    /// Receives the next event, or `None` if the publisher is gone.
    pub async fn recv(&mut self) -> Option<SyncEvent> {
        self.receiver.recv().await.ok()
    }

    /// Tries to receive an event without blocking.
    pub fn try_recv(&mut self) -> Result<SyncEvent, broadcast::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl Clone for EventSubscriber {
    fn clone(&self) -> Self {
        Self {
            receiver: self.receiver.resubscribe(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_publishing() {
        let publisher = EventPublisher::new(10);
        let mut subscriber = publisher.subscribe();

        publisher.publish(SyncEvent::FolderScanCompleted {
            folder: "test".to_string(),
            files_changed: 5,
        });

        let event = subscriber.recv().await;
        assert!(matches!(event, Some(SyncEvent::FolderScanCompleted { .. })));
    }
}
