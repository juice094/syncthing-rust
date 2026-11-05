//! BEP Session state machine
//!
//! Encapsulates the BEP protocol logic after transport handshake:
//! - ClusterConfig exchange
//! - Initial Index transmission
//! - Steady-state message loop (Ping, Index, IndexUpdate, Request, Response, Close)

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tracing::debug;

use syncthing_core::{DeviceId, Identity, Result};

use crate::connection::BepConnection;

/// Observable events emitted by a BEP session at key state transitions.
#[derive(Debug, Clone)]
pub enum BepSessionEvent {
    /// ClusterConfig exchange completed (both directions).
    ClusterConfigComplete {
        device_id: DeviceId,
        shared_folders: Vec<String>,
    },
    /// Initial Index sent to peer.
    IndexSent {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
        last_sequence: u64,
    },
    /// Received full Index from peer.
    IndexReceived {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
    },
    /// Received IndexUpdate from peer.
    IndexUpdateReceived {
        device_id: DeviceId,
        folder: String,
        file_count: usize,
    },
    /// Peer requested a block from us (push direction active).
    BlockRequested {
        device_id: DeviceId,
        folder: String,
        name: String,
        offset: i64,
        size: i32,
    },
    /// Heartbeat timeout detected.
    HeartbeatTimeout {
        device_id: DeviceId,
        last_recv_age: Duration,
    },
    /// Peer index changed; completion state should be re-queried.
    PeerSyncState { device_id: DeviceId, folder: String },
    /// Session ended (clean close or error).
    SessionEnded { device_id: DeviceId, reason: String },
}

/// Per-session counters for observability.
#[derive(Debug, Default)]
pub struct BepSessionMetrics {
    /// BEP messages sent.
    pub messages_sent: AtomicU64,
    /// BEP messages received.
    pub messages_recv: AtomicU64,
    /// Bytes sent.
    pub bytes_sent: AtomicU64,
    /// Bytes received.
    pub bytes_recv: AtomicU64,
    /// Block requests received.
    pub blocks_requested: AtomicU64,
    /// Blocks served.
    pub blocks_served: AtomicU64,
    /// Heartbeat timeouts detected.
    pub heartbeat_timeouts: AtomicU64,
    /// Session errors.
    pub errors: AtomicU64,
    /// Full Indexes received.
    pub index_received: AtomicU64,
    /// IndexUpdates received.
    pub index_update_received: AtomicU64,
    /// Requests received.
    pub requests_received: AtomicU64,
    /// Responses received.
    pub responses_received: AtomicU64,
    /// Close messages received.
    pub closes_received: AtomicU64,
}

/// Handler trait for BEP session events.
///
/// Implementors provide domain-specific logic (e.g., index generation,
/// block serving) without owning the message loop.
#[async_trait::async_trait]
pub trait BepSessionHandler: Send + Sync {
    /// Generate the local ClusterConfig to send to `device_id`.
    async fn generate_cluster_config(
        &self,
        device_id: DeviceId,
    ) -> Result<bep_protocol::messages::ClusterConfig>;

    /// Generate the initial Index for `folder_id` when talking to `device_id`.
    async fn generate_index(
        &self,
        folder_id: &str,
        device_id: DeviceId,
    ) -> Result<syncthing_core::types::Index>;

    /// Remote peer sent a full Index.
    async fn on_index(
        &self,
        device_id: DeviceId,
        index: syncthing_core::types::Index,
    ) -> Result<()>;

    /// Remote peer sent an IndexUpdate.
    async fn on_index_update(
        &self,
        device_id: DeviceId,
        update: syncthing_core::types::IndexUpdate,
    ) -> Result<()>;

    /// Remote peer requested a block. Return the raw block bytes or an error code.
    async fn on_block_request(
        &self,
        device_id: DeviceId,
        req: bep_protocol::messages::Request,
    ) -> std::result::Result<Vec<u8>, bep_protocol::messages::ErrorCode>;
}

/// BEP protocol session for a single connection.
pub struct BepSession {
    /// Remote device identity (abstraction layer).
    // TODO: identity verification / certificate rotation
    #[allow(dead_code)]
    pub(super) identity: Arc<dyn Identity>,
    /// Remote device ID (cached from `identity`).
    pub(super) device_id: DeviceId,
    /// Underlying BEP connection.
    pub(super) conn: Arc<BepConnection>,
    /// Domain-specific event handler.
    pub(super) handler: Arc<dyn BepSessionHandler>,
    /// Pending block request responses keyed by request ID.
    pub(super) pending_responses:
        Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>>,
    /// Optional event subscription channel.
    pub(super) event_tx: Option<tokio::sync::mpsc::Sender<BepSessionEvent>>,
    /// Per-session metrics.
    pub(super) metrics: Arc<BepSessionMetrics>,
    /// Folders shared by the remote peer (known after ClusterConfig).
    pub(super) remote_shared_folders: Option<Vec<String>>,
}

impl BepSession {
    /// Creates a new session without event subscription.
    pub fn new(
        identity: Arc<dyn Identity>,
        conn: Arc<BepConnection>,
        handler: Arc<dyn BepSessionHandler>,
        pending_responses: Arc<
            DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>,
        >,
    ) -> Self {
        let device_id = identity.device_id();
        Self {
            identity,
            device_id,
            conn,
            handler,
            pending_responses,
            event_tx: None,
            metrics: Arc::new(BepSessionMetrics::default()),
            remote_shared_folders: None,
        }
    }

    /// Creates a new session with event subscription.
    pub fn with_events(
        identity: Arc<dyn Identity>,
        conn: Arc<BepConnection>,
        handler: Arc<dyn BepSessionHandler>,
        pending_responses: Arc<
            DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>,
        >,
        event_tx: tokio::sync::mpsc::Sender<BepSessionEvent>,
    ) -> Self {
        let device_id = identity.device_id();
        Self {
            identity,
            device_id,
            conn,
            handler,
            pending_responses,
            event_tx: Some(event_tx),
            metrics: Arc::new(BepSessionMetrics::default()),
            remote_shared_folders: None,
        }
    }

    /// Emits a session event to the subscriber, if any.
    pub(super) fn emit(&self, event: BepSessionEvent) {
        if let Some(ref tx) = self.event_tx {
            if let Err(e) = tx.try_send(event) {
                debug!("Failed to emit BEP session event: {}", e);
            }
        }
    }

    /// Return a clone of the per-session metrics arc.
    pub fn metrics(&self) -> Arc<BepSessionMetrics> {
        Arc::clone(&self.metrics)
    }
}

mod state;

#[cfg(test)]
mod tests;
