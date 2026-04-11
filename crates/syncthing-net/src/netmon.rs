//! Network interface monitor
//!
//! Detects OS network interface changes and emits events.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};
use tracing::debug;

/// Network change event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetChangeEvent {
    /// Network interfaces have changed (added/removed/modified)
    InterfacesChanged,
}

/// Monitors network interface changes
pub struct NetMonitor {
    interface_source: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
}

impl NetMonitor {
    /// Create a new monitor using the real `netdev` interface list
    pub fn new() -> Self {
        Self {
            interface_source: Arc::new(|| {
                netdev::get_interfaces()
                    .into_iter()
                    .map(|iface| iface.name)
                    .collect()
            }),
        }
    }

    /// Create a monitor with a custom interface source (for tests)
    #[cfg(test)]
    pub fn with_source<F>(source: F) -> Self
    where
        F: Fn() -> Vec<String> + Send + Sync + 'static,
    {
        Self {
            interface_source: Arc::new(source),
        }
    }

    /// Subscribe to network change events (polls every 5 seconds)
    pub fn subscribe(&self) -> mpsc::Receiver<NetChangeEvent> {
        self.subscribe_with_interval(Duration::from_secs(5))
    }

    fn subscribe_with_interval(&self, interval_duration: Duration) -> mpsc::Receiver<NetChangeEvent> {
        let (tx, rx) = mpsc::channel(16);
        let source = Arc::clone(&self.interface_source);
        tokio::spawn(async move {
            Self::run(tx, source, interval_duration).await;
        });
        rx
    }

    async fn run(
        tx: mpsc::Sender<NetChangeEvent>,
        source: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
        interval_duration: Duration,
    ) {
        let mut ticker = interval(interval_duration);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut last_interfaces: Option<Vec<String>> = None;

        loop {
            ticker.tick().await;

            let current = source();
            let changed = match &last_interfaces {
                None => true,
                Some(last) => last != &current,
            };

            if changed {
                debug!("Network interfaces changed");
                last_interfaces = Some(current);
                if tx.send(NetChangeEvent::InterfacesChanged).await.is_err() {
                    break;
                }
            }
        }
    }
}

impl Default for NetMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_netmon_detects_interface_change() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = Arc::clone(&call_count);

        let monitor = NetMonitor::with_source(move || {
            let count = cc.fetch_add(1, Ordering::SeqCst);
            match count {
                0 => vec!["eth0".to_string()],
                _ => vec!["eth0".to_string(), "eth1".to_string()],
            }
        });

        let mut rx = monitor.subscribe_with_interval(Duration::from_millis(50));

        // First tick: last_interfaces is None, so we should get an event
        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(event.is_ok());
        assert!(matches!(event.unwrap(), Some(NetChangeEvent::InterfacesChanged)));

        // Second tick: interfaces changed, so we should get another event
        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
        assert!(event.is_ok());
        assert!(matches!(event.unwrap(), Some(NetChangeEvent::InterfacesChanged)));
    }
}
