//! File rename utilities.
//!
//! Provides a Windows-aware atomic rename (exponential backoff retry) and
//! temporary-file path generation.

use crate::error::{Result, SyncError};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{error, warn};

/// Retry configuration.
const RENAME_RETRY_BASE_DELAY_MS: u64 = 1000;
const RENAME_RETRY_MAX_ATTEMPTS: u32 = 5;

/// Builds the temporary file path aligned with `block_server`:
/// `.syncthing.{filename}.tmp`.
pub(crate) fn temp_path_for(file_path: &Path) -> PathBuf {
    let parent = file_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    parent.join(format!(".syncthing.{}.tmp", file_name))
}

/// Windows-aware atomic rename with exponential backoff retries.
///
/// Handles `ERROR_SHARING_VIOLATION` (32) and `ERROR_ACCESS_DENIED` (5) on
/// Windows, where the target file may be locked by another process
/// (antivirus, editors, desktop search, etc.).
///
/// Strategy:
/// 1. direct rename
/// 2. on failure → remove(target) → rename
/// 3. on further failure → exponential backoff retries (1s/2s/4s/8s), at most 5
/// 4. on exhaustion → keep the `.tmp` file and return an error (the next pull
///    cycle retries)
pub(crate) async fn rename_with_retry(
    temp_path: &Path,
    file_path: &Path,
    file_name: &str,
) -> Result<()> {
    match fs::rename(temp_path, file_path).await {
        Ok(()) => return Ok(()),
        Err(e) => {
            warn!(
                file = %file_name,
                error = %e,
                raw_os_error = ?e.raw_os_error(),
                "Initial rename failed, trying remove+rename fallback"
            );
        }
    }

    // Fallback: remove the target first, then rename.
    if file_path.exists() {
        if let Err(e) = fs::remove_file(file_path).await {
            warn!(
                file = %file_name,
                error = %e,
                "Failed to remove target file before rename retry"
            );
        }
    }

    match fs::rename(temp_path, file_path).await {
        Ok(()) => {
            warn!(
                file = %file_name,
                "Rename succeeded after remove fallback"
            );
            return Ok(());
        }
        Err(e) => {
            warn!(
                file = %file_name,
                error = %e,
                raw_os_error = ?e.raw_os_error(),
                "Rename failed after remove fallback, starting exponential backoff"
            );
        }
    }

    let mut delay_ms = RENAME_RETRY_BASE_DELAY_MS;
    for attempt in 1..=RENAME_RETRY_MAX_ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

        // The target may have been unlocked since the last attempt.
        if file_path.exists() {
            let _ = fs::remove_file(file_path).await;
        }

        match fs::rename(temp_path, file_path).await {
            Ok(()) => {
                warn!(
                    file = %file_name,
                    attempt = attempt,
                    "Rename succeeded after retry"
                );
                return Ok(());
            }
            Err(e) => {
                warn!(
                    file = %file_name,
                    attempt = attempt,
                    delay_ms = delay_ms,
                    error = %e,
                    raw_os_error = ?e.raw_os_error(),
                    "Rename retry failed"
                );
            }
        }

        delay_ms *= 2;
    }

    // All retries exhausted: keep the .tmp file for the next pull cycle.
    error!(
        file = %file_name,
        temp = %temp_path.display(),
        target = %file_path.display(),
        "Rename exhausted all retries, preserving temp file for next pull cycle"
    );

    Err(SyncError::pull(
        file_name.to_string(),
        format!(
            "Failed to rename file after {} retries (temp preserved at {})",
            RENAME_RETRY_MAX_ATTEMPTS,
            temp_path.display()
        ),
    ))
}
