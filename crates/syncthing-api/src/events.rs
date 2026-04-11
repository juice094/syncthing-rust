//! Module: syncthing-api
//! Worker: Agent-F
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证

//! WebSocket event system for Syncthing API
//!
//! This module provides an event bus for publishing and subscribing to
//! Syncthing events. Events can be sent to WebSocket clients or used
//! internally within the application.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, trace, warn};

use syncthing_core::types::Event;
use syncthing_core::{Result, SyncthingError};

/// Default channel capacity for event broadcasting
const DEFAULT_CHANNEL_CAPACITY: usize = 1000;

/// Event bus for publishing and subscribing to events
///
/// The event bus uses a broadcast channel to distribute events to all
/// connected subscribers. It also maintains subscription management
/// for WebSocket connections.
///
/// # Example
/// ```
/// use syncthing_api::events::EventBus;
/// use syncthing_core::types::{Event, FolderId};
///
/// #[tokio::main]
/// async fn main() {
///     let event_bus = EventBus::new();
///
///     // Subscribe to events
///     let mut rx = event_bus.subscribe();
///
///     // Publish an event
///     event_bus.publish(Event::LocalIndexUpdated {
///         folder: FolderId::new("default"),
///         items: vec!["file.txt".to_string()],
///     });
///
///     // Receive the event
///     let event = rx.recv().await.unwrap();
///     assert!(matches!(event, Event::LocalIndexUpdated { .. }));
/// }
/// ```
#[derive(Debug, Clone)]
pub struct EventBus {
    /// Broadcast sender for events
    sender: broadcast::Sender<Event>,
    /// Active WebSocket connections
    connections: Arc<RwLock<HashMap<String, mpsc::Sender<Event>>>>,
}

impl EventBus {
    /// Create a new event bus
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_CHANNEL_CAPACITY);
        Self {
            sender,
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new event bus with custom capacity
    ///
    /// # Arguments
    /// * `capacity` - Maximum number of events to buffer
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Subscribe to events
    ///
    /// Returns a broadcast receiver that receives all events published
    /// after the subscription is created.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// Publish an event to all subscribers
    ///
    /// # Arguments
    /// * `event` - The event to publish
    pub fn publish(&self, event: Event) {
        trace!("Publishing event: {:?}", event);

        // Send to broadcast channel
        if let Err(e) = self.sender.send(event.clone()) {
            warn!("Failed to broadcast event: {} (no active receivers)", e);
        }

        // Send to WebSocket connections
        let connections = self.connections.clone();
        let event_clone = event.clone();
        tokio::spawn(async move {
            let conns = connections.read().await;
            for (id, tx) in conns.iter() {
                if let Err(e) = tx.try_send(event_clone.clone()) {
                    debug!("Failed to send event to connection {}: {}", id, e);
                }
            }
        });
    }

    /// Register a WebSocket connection
    ///
    /// # Arguments
    /// * `id` - Unique connection identifier
    /// * `sender` - Channel sender for sending events to the connection
    pub async fn register_connection(
        &self,
        id: String,
        sender: mpsc::Sender<Event>,
    ) -> Result<()> {
        let mut connections = self.connections.write().await;
        connections.insert(id.clone(), sender);
        debug!("Registered WebSocket connection: {}", id);
        Ok(())
    }

    /// Unregister a WebSocket connection
    ///
    /// # Arguments
    /// * `id` - Connection identifier to remove
    pub async fn unregister_connection(&self, id: &str) -> Result<()> {
        let mut connections = self.connections.write().await;
        connections.remove(id);
        debug!("Unregistered WebSocket connection: {}", id);
        Ok(())
    }

    /// Get the number of active connections
    pub async fn connection_count(&self) -> usize {
        let connections = self.connections.read().await;
        connections.len()
    }

    /// Get the number of broadcast receivers
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Event subscriber with filtered events
///
/// Allows subscribing to specific event types only.
pub struct FilteredSubscriber {
    /// Underlying broadcast receiver
    receiver: broadcast::Receiver<Event>,
    /// Event types to filter for (empty = all events)
    filter: Vec<EventType>,
}

/// Event type for filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    /// Folder summary events
    FolderSummary,
    /// Item finished events
    ItemFinished,
    /// Device connected events
    DeviceConnected,
    /// Device disconnected events
    DeviceDisconnected,
    /// Local index updated events
    LocalIndexUpdated,
    /// Remote index updated events
    RemoteIndexUpdated,
}

impl FilteredSubscriber {
    /// Create a new filtered subscriber
    ///
    /// # Arguments
    /// * `event_bus` - The event bus to subscribe to
    /// * `filter` - Event types to receive (empty for all)
    pub fn new(event_bus: &EventBus, filter: Vec<EventType>) -> Self {
        Self {
            receiver: event_bus.subscribe(),
            filter,
        }
    }

    /// Receive the next event
    ///
    /// If a filter is set, events not matching the filter are skipped.
    pub async fn recv(&mut self) -> Result<Event> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    if self.filter.is_empty() || self.matches_filter(&event) {
                        return Ok(event);
                    }
                    // Event doesn't match filter, continue waiting
                }
                Err(e) => {
                    return Err(SyncthingError::internal(format!(
                        "Event receiver error: {}",
                        e
                    )));
                }
            }
        }
    }

    /// Try to receive an event without blocking
    pub fn try_recv(&mut self) -> Result<Option<Event>> {
        loop {
            match self.receiver.try_recv() {
                Ok(event) => {
                    if self.filter.is_empty() || self.matches_filter(&event) {
                        return Ok(Some(event));
                    }
                    // Event doesn't match filter, try next
                }
                Err(broadcast::error::TryRecvError::Empty) => return Ok(None),
                Err(broadcast::error::TryRecvError::Closed) => {
                    return Err(SyncthingError::internal(
                        "Event channel closed".to_string(),
                    ))
                }
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    warn!("Event subscriber lagged by {} events", n);
                    continue;
                }
            }
        }
    }

    /// Check if an event matches the filter
    fn matches_filter(&self, event: &Event) -> bool {
        let event_type = match event {
            Event::FolderSummary { .. } => EventType::FolderSummary,
            Event::ItemFinished { .. } => EventType::ItemFinished,
            Event::DeviceConnected { .. } => EventType::DeviceConnected,
            Event::DeviceDisconnected { .. } => EventType::DeviceDisconnected,
            Event::LocalIndexUpdated { .. } => EventType::LocalIndexUpdated,
            Event::RemoteIndexUpdated { .. } => EventType::RemoteIndexUpdated,
        };
        self.filter.contains(&event_type)
    }
}

/// Event statistics tracker
#[derive(Debug, Default, Clone)]
pub struct EventStats {
    /// Total events published
    pub total_published: u64,
    /// Events by type
    pub by_type: HashMap<EventType, u64>,
}

impl EventStats {
    /// Create new event stats
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an event
    pub fn record(&mut self, event: &Event) {
        self.total_published += 1;
        let event_type = match event {
            Event::FolderSummary { .. } => EventType::FolderSummary,
            Event::ItemFinished { .. } => EventType::ItemFinished,
            Event::DeviceConnected { .. } => EventType::DeviceConnected,
            Event::DeviceDisconnected { .. } => EventType::DeviceDisconnected,
            Event::LocalIndexUpdated { .. } => EventType::LocalIndexUpdated,
            Event::RemoteIndexUpdated { .. } => EventType::RemoteIndexUpdated,
        };
        *self.by_type.entry(event_type).or_insert(0) += 1;
    }
}

/// Event bus with statistics tracking
#[derive(Debug, Clone)]
pub struct InstrumentedEventBus {
    /// Inner event bus
    inner: EventBus,
    /// Statistics
    stats: Arc<RwLock<EventStats>>,
}

impl InstrumentedEventBus {
    /// Create a new instrumented event bus
    pub fn new() -> Self {
        Self {
            inner: EventBus::new(),
            stats: Arc::new(RwLock::new(EventStats::new())),
        }
    }

    /// Publish an event and record statistics
    pub fn publish(&self, event: Event) {
        // Record stats
        let stats = self.stats.clone();
        let event_clone = event.clone();
        tokio::spawn(async move {
            let mut s = stats.write().await;
            s.record(&event_clone);
        });

        // Publish to inner bus
        self.inner.publish(event);
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.subscribe()
    }

    /// Get current statistics
    pub async fn stats(&self) -> EventStats {
        self.stats.read().await.clone()
    }

    /// Register a WebSocket connection
    pub async fn register_connection(
        &self,
        id: String,
        sender: mpsc::Sender<Event>,
    ) -> Result<()> {
        self.inner.register_connection(id, sender).await
    }

    /// Unregister a WebSocket connection
    pub async fn unregister_connection(&self, id: &str) -> Result<()> {
        self.inner.unregister_connection(id).await
    }
}

impl Default for InstrumentedEventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_core::types::{FolderId, FolderSummary};

    #[tokio::test]
    async fn test_event_bus_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let event = Event::LocalIndexUpdated {
            folder: FolderId::new("test"),
            items: vec!["file.txt".to_string()],
        };

        bus.publish(event.clone());

        let received = rx.recv().await.unwrap();
        match received {
            Event::LocalIndexUpdated { folder, items } => {
                assert_eq!(folder.as_str(), "test");
                assert_eq!(items, vec!["file.txt".to_string()]);
            }
            _ => panic!("Wrong event type"),
        }
    }

    #[tokio::test]
    async fn test_event_bus_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        let event = Event::FolderSummary {
            folder: FolderId::new("test"),
            summary: FolderSummary::default(),
        };

        bus.publish(event);

        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert!(matches!(received1, Event::FolderSummary { .. }));
        assert!(matches!(received2, Event::FolderSummary { .. }));
    }

    #[tokio::test]
    async fn test_filtered_subscriber() {
        let bus = EventBus::new();
        let mut subscriber = FilteredSubscriber::new(
            &bus,
            vec![EventType::FolderSummary, EventType::ItemFinished],
        );

        // Publish a filtered event
        bus.publish(Event::FolderSummary {
            folder: FolderId::new("test"),
            summary: FolderSummary::default(),
        });

        // Should receive the event
        let received = subscriber.recv().await.unwrap();
        assert!(matches!(received, Event::FolderSummary { .. }));

        // Publish a non-filtered event
        bus.publish(Event::LocalIndexUpdated {
            folder: FolderId::new("test"),
            items: vec![],
        });

        // This would block forever in recv(), so we use try_recv
        let not_received = subscriber.try_recv().unwrap();
        assert!(not_received.is_none());
    }

    #[tokio::test]
    async fn test_websocket_connection_management() {
        let bus = EventBus::new();
        let (tx, mut rx) = mpsc::channel(10);

        // Register connection
        bus.register_connection("conn1".to_string(), tx).await.unwrap();
        assert_eq!(bus.connection_count().await, 1);

        // Publish event
        bus.publish(Event::DeviceConnected {
            device: syncthing_core::DeviceId::from_bytes(&[0u8; 32]).unwrap(),
            addr: "127.0.0.1:22000".to_string(),
        });

        // Should receive on WebSocket channel
        let received = rx.recv().await;
        assert!(received.is_some());

        // Unregister connection
        bus.unregister_connection("conn1").await.unwrap();
        assert_eq!(bus.connection_count().await, 0);
    }

    #[tokio::test]
    async fn test_instrumented_event_bus() {
        let bus = InstrumentedEventBus::new();

        bus.publish(Event::FolderSummary {
            folder: FolderId::new("test"),
            summary: FolderSummary::default(),
        });

        bus.publish(Event::ItemFinished {
            folder: FolderId::new("test"),
            item: "file.txt".to_string(),
            error: None,
        });

        // Give stats time to update
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let stats = bus.stats().await;
        assert_eq!(stats.total_published, 2);
        assert_eq!(stats.by_type.get(&EventType::FolderSummary), Some(&1));
        assert_eq!(stats.by_type.get(&EventType::ItemFinished), Some(&1));
    }
}
