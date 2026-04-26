//! 文件系统监视器
//!
//! 基于 `notify` crate 实现秒级文件变更检测，替代长轮询扫描。

use crate::error::{Result, SyncError};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// 文件夹文件系统监视器
pub struct FolderWatcher;

impl FolderWatcher {
    /// 启动对指定路径的递归监听。
    ///
    /// 返回 watcher 和事件接收器。调用者需在异步循环中读取事件、
    /// 实现去抖动，并在 drop watcher 时自动停止监听。
    pub fn watch(
        folder_id: &str,
        path: &str,
    ) -> Result<(RecommendedWatcher, mpsc::UnboundedReceiver<Event>)> {
        let watch_path = std::path::PathBuf::from(path);
        if !watch_path.exists() {
            return Err(SyncError::scan(
                folder_id.to_string(),
                format!("Watch path does not exist: {}", path),
            ));
        }

        let (tx, rx) = mpsc::unbounded_channel::<Event>();
        let folder_id_for_log = folder_id.to_string();
        let folder_id_for_err = folder_id.to_string();

        let mut watcher: RecommendedWatcher = RecommendedWatcher::new(
            move |res: std::result::Result<Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        // 过滤掉纯访问事件，减少噪音
                        match event.kind {
                            notify::EventKind::Access(_) => {}
                            _ => {
                                let _ = tx.send(event);
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
                SyncError::scan(
                    folder_id_watch,
                    format!("Failed to watch path: {}", e),
                )
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
    async fn test_watcher_detects_file_creation() {
        // 创建临时目录
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().to_str().unwrap();

        // 启动 watcher
        let (_watcher, mut rx) = FolderWatcher::watch("test-folder", path).unwrap();

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
