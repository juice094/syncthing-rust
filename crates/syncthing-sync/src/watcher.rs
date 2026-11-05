//! Filesystem watcher.
//!
//! Provides sub-second file-change detection on top of the `notify` crate,
//! replacing long-polling scans.

use crate::error::{Result, SyncError};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Recursive filesystem watcher for one folder.
pub struct FolderWatcher;

impl FolderWatcher {
    /// Starts recursively watching the given path.
    ///
    /// Returns the watcher and an event receiver. The caller must read events
    /// in an async loop, implement debouncing, and drop the watcher to stop
    /// watching. The `dropped` counter is incremented whenever the watcher
    /// callback drops an event because the channel is full.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Scan`] when the path does not exist, watcher
    /// creation fails, or the path cannot be watched.
    pub fn watch(
        folder_id: &str,
        path: &str,
        dropped: Arc<AtomicU64>,
    ) -> Result<(RecommendedWatcher, mpsc::Receiver<Event>)> {
        let watch_path = std::path::PathBuf::from(path);
        if !watch_path.exists() {
            return Err(SyncError::scan(
                folder_id.to_string(),
                format!("Watch path does not exist: {}", path),
            ));
        }

        let (tx, rx) = mpsc::channel::<Event>(256);
        let dropped_for_cb = dropped.clone();
        let folder_id_for_log = folder_id.to_string();
        let folder_id_for_err = folder_id.to_string();

        let mut watcher: RecommendedWatcher = RecommendedWatcher::new(
            move |res: std::result::Result<Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        // Skip pure-access events to reduce noise.
                        match event.kind {
                            notify::EventKind::Access(_) => {}
                            _ => {
                                if let Err(e) = tx.try_send(event) {
                                    dropped_for_cb.fetch_add(1, Ordering::Relaxed);
                                    warn!(folder_id = %folder_id_for_log, error = %e, "Watcher event dropped (channel full)");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(folder_id = %folder_id_for_log, error = %e, "Watcher error");
                    }
                }
            },
            Config::default(),
        )
        .map_err(|e| {
            SyncError::scan(
                folder_id_for_err.clone(),
                format!("Failed to create watcher: {}", e),
            )
        })?;

        let folder_id_watch = folder_id.to_string();
        watcher
            .watch(&watch_path, RecursiveMode::Recursive)
            .map_err(|e| {
                SyncError::scan(folder_id_watch, format!("Failed to watch path: {}", e))
            })?;

        info!(folder_id = %folder_id, path = %path, "Folder watcher started");

        Ok((watcher, rx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "flaky FSEvents latency on macOS CI runners"
    )]
    async fn test_watcher_detects_file_creation() {
        // 创建临时目录
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap();

        // 启动 watcher
        let (_watcher, mut rx) =
            FolderWatcher::watch("test-folder", path, std::sync::Arc::new(AtomicU64::new(0)))
                .unwrap();

        // 短暂等待 watcher 初始化（Windows 需要）
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 创建文件
        let file_path = temp_dir.path().join("test_file.txt");
        {
            let mut file = std::fs::File::create(&file_path).unwrap();
            file.write_all(b"hello").unwrap();
        }

        // 等待事件（最多 5 秒）
        let event = timeout(Duration::from_secs(5), rx.recv()).await;

        assert!(
            event.is_ok(),
            "Watcher did not detect file creation within 5s"
        );
        let event = event.unwrap().expect("Channel closed unexpectedly");
        assert!(
            matches!(event.kind, notify::EventKind::Create(_)),
            "Expected Create event, got {:?}",
            event.kind
        );
        assert!(
            event.paths.contains(&file_path),
            "Event path mismatch: expected {:?}, got {:?}",
            file_path,
            event.paths
        );
    }
}
