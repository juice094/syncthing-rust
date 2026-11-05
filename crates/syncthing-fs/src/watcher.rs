//! Filesystem change watcher.
//!
//! Wraps the `notify` crate: converts native events into `FsEvent`,
//! supports debounced batching (`with_debounce`) and a shared-buffer
//! helper (`create_buffer_watcher`) for polling-style consumers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{Config, Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher};
use syncthing_core::{Result, SyncthingError};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

/// Filesystem event types
///
/// Represents the different types of changes that can occur
/// in the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsEvent {
    /// File or directory was created
    Created(PathBuf),
    /// File or directory was modified
    Modified(PathBuf),
    /// File or directory was removed
    Removed(PathBuf),
    /// File or directory was renamed
    Renamed {
        /// Original path
        from: PathBuf,
        /// New path
        to: PathBuf,
    },
}

impl FsEvent {
    /// Get the path(s) associated with this event
    pub fn paths(&self) -> Vec<&Path> {
        match self {
            FsEvent::Created(p) => vec![p],
            FsEvent::Modified(p) => vec![p],
            FsEvent::Removed(p) => vec![p],
            FsEvent::Renamed { from, to } => vec![from, to],
        }
    }

    /// Check if this event affects the given path
    pub fn affects(&self, path: &Path) -> bool {
        self.paths().contains(&path)
    }
}

/// Folder watcher for monitoring filesystem changes
///
/// Watches a folder and its subdirectories for changes,
/// reporting events through a callback function.
pub struct FolderWatcher {
    /// Channel sender for passing events from notify to the async task
    _watcher: RecommendedWatcher,
    /// Handle to the background task processing events
    _task_handle: tokio::task::JoinHandle<()>,
}

impl FolderWatcher {
    /// Create a new folder watcher
    ///
    /// Watches the given path recursively and calls the callback
    /// for each filesystem event.
    ///
    /// # Arguments
    /// * `path` - The directory to watch
    /// * `callback` - Function to call when filesystem events occur
    ///
    /// # Returns
    /// A `FolderWatcher` that will continue watching until dropped
    ///
    /// # Errors
    /// Returns an error if the watcher cannot be created or the path cannot
    /// be watched.
    ///
    /// # Example
    /// ```
    /// use syncthing_fs::watcher::{FolderWatcher, FsEvent};
    /// use std::path::Path;
    ///
    /// # async fn example() -> syncthing_core::Result<()> {
    /// let watcher = FolderWatcher::new(Path::new("/path/to/watch"), |event| {
    ///     println!("Filesystem event: {:?}", event);
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(path: &Path, callback: impl Fn(FsEvent) + Send + 'static) -> Result<Self> {
        let path = path.to_path_buf();

        let (tx, mut rx) = mpsc::channel::<notify::Result<NotifyEvent>>(1024);

        let watcher = {
            let tx = tx;
            RecommendedWatcher::new(
                move |res| {
                    if let Err(e) = tx.try_send(res) {
                        error!("Failed to send filesystem event: {}", e);
                    }
                },
                Config::default()
                    .with_poll_interval(Duration::from_secs(1))
                    .with_compare_contents(false),
            )
            .map_err(|e| SyncthingError::io(format!("Failed to create watcher: {}", e)))?
        };

        let mut watcher = watcher;

        watcher
            .watch(&path, RecursiveMode::Recursive)
            .map_err(|e| SyncthingError::io(format!("Failed to watch path: {}", e)))?;

        debug!("Started watching: {}", path.display());

        let task_handle = tokio::spawn(async move {
            while let Some(result) = rx.recv().await {
                match result {
                    Ok(event) => {
                        for fs_event in convert_event(event) {
                            callback(fs_event);
                        }
                    }
                    Err(e) => {
                        warn!("Filesystem watcher error: {}", e);
                    }
                }
            }
            debug!("Filesystem watcher task ended");
        });

        Ok(FolderWatcher {
            _watcher: watcher,
            _task_handle: task_handle,
        })
    }

    /// Create a new watcher with debouncing
    ///
    /// Multiple events for the same path within the debounce interval
    /// will be coalesced into a single event.
    ///
    /// # Arguments
    /// * `path` - The directory to watch
    /// * `debounce_ms` - Debounce interval in milliseconds
    /// * `callback` - Function to call when filesystem events occur
    ///
    /// # Errors
    /// Returns an error if the watcher cannot be created or the path cannot
    /// be watched.
    pub async fn with_debounce(
        path: &Path,
        debounce_ms: u64,
        callback: impl Fn(Vec<FsEvent>) + Send + 'static,
    ) -> Result<Self> {
        let path = path.to_path_buf();
        let debounce_duration = Duration::from_millis(debounce_ms);

        let (tx, mut rx) = mpsc::channel::<notify::Result<NotifyEvent>>(1024);

        let watcher = {
            let tx = tx;
            RecommendedWatcher::new(
                move |res| {
                    if let Err(e) = tx.try_send(res) {
                        error!("Failed to send filesystem event: {}", e);
                    }
                },
                Config::default()
                    .with_poll_interval(Duration::from_secs(1))
                    .with_compare_contents(false),
            )
            .map_err(|e| SyncthingError::io(format!("Failed to create watcher: {}", e)))?
        };

        let mut watcher = watcher;
        watcher
            .watch(&path, RecursiveMode::Recursive)
            .map_err(|e| SyncthingError::io(format!("Failed to watch path: {}", e)))?;

        let task_handle = tokio::spawn(async move {
            let mut pending_events: Vec<FsEvent> = Vec::new();
            let mut last_event_time = tokio::time::Instant::now();

            loop {
                let timeout = tokio::time::sleep(debounce_duration);
                tokio::pin!(timeout);

                tokio::select! {
                    result = rx.recv() => {
                        match result {
                            Some(Ok(event)) => {
                                let events = convert_event(event);
                                pending_events.extend(events);
                                last_event_time = tokio::time::Instant::now();
                            }
                            Some(Err(e)) => {
                                warn!("Filesystem watcher error: {}", e);
                            }
                            None => break,
                        }
                    }
                    _ = &mut timeout => {
                        if !pending_events.is_empty() &&
                           last_event_time.elapsed() >= debounce_duration {
                            callback(std::mem::take(&mut pending_events));
                        }
                    }
                }
            }
        });

        Ok(FolderWatcher {
            _watcher: watcher,
            _task_handle: task_handle,
        })
    }
}

/// Convert a notify event to our FsEvent type
///
/// Handles the conversion from notify's event format to our
/// simplified event format. Access events are dropped; events with
/// unclear kinds are inferred from path existence.
fn convert_event(event: NotifyEvent) -> Vec<FsEvent> {
    use notify::EventKind;

    let paths: Vec<_> = event.paths.into_iter().collect();

    match event.kind {
        EventKind::Create(_) => paths.into_iter().map(FsEvent::Created).collect(),
        EventKind::Modify(_) => paths.into_iter().map(FsEvent::Modified).collect(),
        EventKind::Remove(_) => paths.into_iter().map(FsEvent::Removed).collect(),
        EventKind::Access(_) => vec![],
        EventKind::Any | EventKind::Other => paths
            .into_iter()
            .map(|p| {
                if p.exists() {
                    FsEvent::Modified(p)
                } else {
                    FsEvent::Removed(p)
                }
            })
            .collect(),
    }
}

/// Batch event collector for efficient processing
///
/// Collects events and provides them in batches.
pub struct EventCollector {
    events: Vec<FsEvent>,
}

impl EventCollector {
    /// Create a new event collector
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Add an event to the collection
    ///
    /// A `Created` event cancels a pending `Removed` event for the same path;
    /// a `Removed` event cancels all pending events affecting that path.
    pub fn add(&mut self, event: FsEvent) {
        match &event {
            FsEvent::Created(path) => {
                self.events
                    .retain(|e| !matches!(e, FsEvent::Removed(p) if p == path));
            }
            FsEvent::Removed(path) => {
                self.events.retain(|e| !e.affects(path));
            }
            _ => {}
        }

        self.events.push(event);
    }

    /// Take all collected events
    pub fn take_all(&mut self) -> Vec<FsEvent> {
        std::mem::take(&mut self.events)
    }

    /// Check if collector is empty
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Get the number of collected events
    pub fn len(&self) -> usize {
        self.events.len()
    }
}

impl Default for EventCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a watcher that collects events into a shared buffer
///
/// Useful when you want to poll for events rather than
/// receive them via callback.
///
/// # Errors
/// Returns an error if the watcher cannot be created or the path cannot
/// be watched.
pub async fn create_buffer_watcher(
    path: &Path,
) -> Result<(FolderWatcher, Arc<tokio::sync::Mutex<Vec<FsEvent>>>)> {
    let buffer = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let buffer_clone = buffer.clone();

    let watcher = FolderWatcher::new(path, move |event| {
        if let Ok(mut guard) = buffer_clone.try_lock() {
            guard.push(event);
        }
    })
    .await?;

    Ok((watcher, buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::fs;
    use tokio::time::{sleep, timeout, Duration};

    #[test]
    fn test_fs_event_paths() {
        let created = FsEvent::Created(PathBuf::from("/test/file.txt"));
        assert_eq!(created.paths(), vec![Path::new("/test/file.txt")]);

        let renamed = FsEvent::Renamed {
            from: PathBuf::from("/test/old.txt"),
            to: PathBuf::from("/test/new.txt"),
        };
        assert_eq!(
            renamed.paths(),
            vec![Path::new("/test/old.txt"), Path::new("/test/new.txt")]
        );
    }

    #[test]
    fn test_fs_event_affects() {
        let event = FsEvent::Modified(PathBuf::from("/test/file.txt"));
        assert!(event.affects(Path::new("/test/file.txt")));
        assert!(!event.affects(Path::new("/test/other.txt")));
    }

    #[tokio::test]
    async fn test_event_collector() {
        let mut collector = EventCollector::new();

        collector.add(FsEvent::Created(PathBuf::from("/a.txt")));
        collector.add(FsEvent::Modified(PathBuf::from("/b.txt")));

        assert_eq!(collector.len(), 2);

        let events = collector.take_all();
        assert_eq!(events.len(), 2);
        assert!(collector.is_empty());
    }

    #[tokio::test]
    async fn test_event_collector_dedup() {
        let mut collector = EventCollector::new();

        // Add created event
        collector.add(FsEvent::Created(PathBuf::from("/file.txt")));
        assert_eq!(collector.len(), 1);

        // Add removed event for same file - should clear the created event
        collector.add(FsEvent::Removed(PathBuf::from("/file.txt")));
        // Now we have only the removed event
        assert_eq!(collector.len(), 1);

        let events = collector.take_all();
        assert!(matches!(events[0], FsEvent::Removed(_)));
    }

    #[tokio::test]
    async fn test_folder_watcher_create_file() {
        let temp_dir = TempDir::new().unwrap();
        let event_count = Arc::new(AtomicUsize::new(0));
        let count_clone = event_count.clone();

        let _watcher = FolderWatcher::new(temp_dir.path(), move |event| {
            if matches!(event, FsEvent::Created(_)) {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await
        .unwrap();

        // Give watcher time to start
        sleep(Duration::from_millis(100)).await;

        // Create a file
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, b"content").await.unwrap();

        // Wait for event
        let result = timeout(Duration::from_secs(2), async {
            loop {
                if event_count.load(Ordering::SeqCst) > 0 {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;

        assert!(result.is_ok(), "Should have received a create event");
    }

    #[tokio::test]
    async fn test_folder_watcher_modify_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, b"initial").await.unwrap();

        let event_count = Arc::new(AtomicUsize::new(0));
        let count_clone = event_count.clone();

        let _watcher = FolderWatcher::new(temp_dir.path(), move |event| {
            if matches!(event, FsEvent::Modified(_)) {
                count_clone.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await
        .unwrap();

        // Give watcher time to start
        sleep(Duration::from_millis(100)).await;

        // Modify the file
        fs::write(&file_path, b"modified").await.unwrap();

        // Wait for event
        let result = timeout(Duration::from_secs(2), async {
            loop {
                if event_count.load(Ordering::SeqCst) > 0 {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;

        assert!(result.is_ok(), "Should have received a modify event");
    }

    #[tokio::test]
    async fn test_buffer_watcher() {
        let temp_dir = TempDir::new().unwrap();
        let (_watcher, buffer) = create_buffer_watcher(temp_dir.path()).await.unwrap();

        // Give watcher time to start
        sleep(Duration::from_millis(100)).await;

        // Create a file
        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, b"content").await.unwrap();

        // Wait for event to be collected
        let result = timeout(Duration::from_secs(2), async {
            loop {
                let events = buffer.lock().await;
                if !events.is_empty() {
                    break;
                }
                drop(events);
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;

        assert!(result.is_ok(), "Should have received an event");
        assert!(!buffer.lock().await.is_empty());
    }
}
