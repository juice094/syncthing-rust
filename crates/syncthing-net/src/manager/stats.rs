use dashmap::DashMap;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use syncthing_core::{DeviceId, RetryConfig};

/// Aggregated manager statistics.
#[derive(Debug, Clone)]
pub struct ManagerStats {
    /// Number of currently active connections.
    pub active_connections: usize,
    /// Number of connected devices.
    pub connected_devices: usize,
    /// Number of pending connection attempts.
    pub pending_connections: usize,
    /// Total bytes sent across all connections.
    pub total_bytes_sent: u64,
    /// Total bytes received across all connections.
    pub total_bytes_received: u64,
}

/// Reconnect scheduler with per-device backoff.
pub struct ReconnectScheduler {
    config: RetryConfig,
    pending: DashMap<DeviceId, JoinHandle<()>>,
}

impl ReconnectScheduler {
    /// Creates a scheduler using the given retry configuration.
    pub fn new(config: RetryConfig) -> Self {
        Self {
            config,
            pending: DashMap::new(),
        }
    }

    /// Schedules a reconnect task for the device after backoff, replacing any
    /// existing scheduled task for the same device.
    pub fn schedule<F>(&self, device_id: DeviceId, attempt: u32, task: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        if let Some((_, handle)) = self.pending.remove(&device_id) {
            handle.abort();
        }

        let backoff = self.config.backoff_duration(attempt);
        let handle = tokio::spawn(async move {
            sleep(backoff).await;
            task.await;
        });

        self.pending.insert(device_id, handle);
    }

    /// Cancels the scheduled reconnect task for the device, if any.
    pub fn cancel(&self, device_id: &DeviceId) {
        if let Some((_, handle)) = self.pending.remove(device_id) {
            handle.abort();
        }
    }
}
