use tokio::time::interval;
use tracing::{debug, info, warn};

use syncthing_core::DeviceId;

use crate::connection::ConnectionEvent;

use super::ConnectionManager;

impl ConnectionManager {
    /// Spawns the connection event handling task.
    pub(crate) fn spawn_event_handler(&self) {
        let mut event_rx = match self.event_rx.write().take() {
            Some(rx) => rx,
            None => {
                warn!("Event handler already spawned, skipping");
                return;
            }
        };

        let weak = match self.self_weak() {
            Ok(w) => w,
            Err(e) => {
                warn!("Failed to get self_weak, aborting event handler: {}", e);
                return;
            }
        };

        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                if let Some(manager) = weak.upgrade() {
                    manager.handle_event(event).await;
                } else {
                    break;
                }
            }
        });
    }

    /// Handles a connection event.
    async fn handle_event(&self, event: ConnectionEvent) {
        match event {
            ConnectionEvent::Connected { device_id } => {
                debug!("Device {} connected", device_id);
            }
            ConnectionEvent::Disconnected { reason } => {
                // The event carries no device ID, so the block below is
                // currently unreachable; device teardown goes through
                // `disconnect` / `disconnect_connection` instead.
                let _device_id: Option<DeviceId> = None;
                info!("Device disconnected: {:?} - {}", _device_id, reason);

                if let Some(ref d) = _device_id {
                    if self.should_reconnect(d, &reason) {
                        self.schedule_reconnect(*d).await;
                    }

                    if let Some(callback) = self.on_disconnected.read().as_ref() {
                        callback(*d, reason.clone());
                    }
                }
            }
            ConnectionEvent::Error { error } => {
                warn!("Connection error: {}", error);
            }
            _ => {}
        }
    }

    /// Spawns the periodic maintenance task (stale connection cleanup).
    pub(crate) fn spawn_maintenance_task(&self) {
        let interval_duration = self.config.heartbeat_interval;

        let weak = match self.self_weak() {
            Ok(w) => w,
            Err(e) => {
                warn!("Failed to get self_weak, aborting maintenance task: {}", e);
                return;
            }
        };

        let handle = tokio::spawn(async move {
            let mut ticker = interval(interval_duration);

            loop {
                ticker.tick().await;
                if let Some(manager) = weak.upgrade() {
                    manager.cleanup_stale_connections().await;
                } else {
                    break;
                }
            }
        });

        *self.maintenance_handle.write() = Some(handle);
    }
}
