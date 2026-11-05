//! Helpers for running synchronous sled I/O on tokio's blocking thread pool.
//!
//! See "docs/design/ADR-20261105-db-blocking-io.md".

use std::time::Duration;

use syncthing_core::{Result, SyncthingError};

const BLOCKING_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs a synchronous closure on tokio's blocking thread pool, guarded by a timeout.
///
/// Used to keep sled's synchronous disk I/O off the async runtime's worker
/// threads so a slow disk cannot stall the event loop.
///
/// # Errors
/// Returns a timeout error if the closure does not complete within 10 seconds,
/// or an internal error if the closure panics.
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
