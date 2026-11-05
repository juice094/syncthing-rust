use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use syncthing_core::DeviceId;
use tokio::sync::oneshot;

use crate::connection::BepConnection;

/// Connection pool entry.
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct ConnectionEntry {
    /// The connection.
    pub(crate) conn: Arc<BepConnection>,
    /// When the connection was established.
    pub(crate) connected_at: Instant,
    /// Retry count at establishment time.
    pub(crate) retry_count: u32,
}

impl ConnectionEntry {
    /// Creates a new entry for a freshly established connection.
    pub(crate) fn new(conn: Arc<BepConnection>) -> Self {
        Self {
            conn,
            connected_at: Instant::now(),
            retry_count: 0,
        }
    }

    /// Returns whether the connection has no recent activity beyond `timeout`.
    pub(crate) fn is_stale(&self, timeout: std::time::Duration) -> bool {
        self.conn
            .last_activity_age()
            .is_none_or(|age| age > timeout)
    }
}

/// Device with a connection attempt in progress.
#[allow(dead_code)]
pub(crate) struct PendingConnection {
    pub(crate) device_id: DeviceId,
    pub(crate) addresses: Vec<SocketAddr>,
    pub(crate) relay_urls: Vec<String>,
    pub(crate) retry_count: u32,
    pub(crate) last_attempt: Option<Instant>,
    /// Cancellation handle for the retry task.
    pub(crate) _cancel_tx: Option<oneshot::Sender<()>>,
}
