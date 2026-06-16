//! 将同步 sled I/O 放到独立阻塞线程的工具函数

use std::time::Duration;

use syncthing_core::{Result, SyncthingError};

const BLOCKING_TIMEOUT: Duration = Duration::from_secs(10);

/// 在 tokio 的阻塞线程池中执行同步闭包，并加超时保护。
///
/// 用于把 sled 等同步磁盘 I/O 从 async runtime 工作线程上挪走，
/// 避免阻塞 tokio 的事件循环。
pub async fn run_blocking<F, T>(name: &'static str, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::time::timeout(BLOCKING_TIMEOUT, tokio::task::spawn_blocking(f))
        .await
        .map_err(|_| SyncthingError::timeout(format!("{}: blocking operation timed out", name)))?
        .map_err(|e| SyncthingError::Internal(format!("blocking task panicked: {}", e)))?
}
