//! 文件系统监视器
//!
//! 基于 `notify` crate 实现秒级文件变更检测，替代长轮询扫描。

use crate::error::{Result, SyncError};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

const DEBOUNCE_DURATION: Duration = Duration::from_secs(1);

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
