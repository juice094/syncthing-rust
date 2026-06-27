//! 文件重命名工具
//!
//! 提供 Windows-aware 原子重命名（指数退避重试）和临时文件路径生成。

use crate::error::{Result, SyncError};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{error, warn};

/// 重试配置
const RENAME_RETRY_BASE_DELAY_MS: u64 = 1000;
const RENAME_RETRY_MAX_ATTEMPTS: u32 = 5;

/// 生成与 block_server 对齐的临时文件路径
/// 格式: `.syncthing.{filename}.tmp`
pub(crate) fn temp_path_for(file_path: &Path) -> PathBuf {
    let parent = file_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    parent.join(format!(".syncthing.{}.tmp", file_name))
}

/// Windows-aware 原子重命名，带指数退避重试
///
/// 处理 Windows 上目标文件被其他进程锁定（杀毒软件、编辑器、桌面搜索等）
/// 导致的 `ERROR_SHARING_VIOLATION` (32) 和 `ERROR_ACCESS_DENIED` (5)。
///
/// 策略:
/// 1. 直接 rename
/// 2. 失败 → remove(target) → rename
/// 3. 仍失败 → 指数退避重试 (1s/2s/4s/8s)，最多 5 次
/// 4. 最终失败 → 保留 .tmp，返回错误（下次 pull 周期再试）
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

    // Fallback: 先删目标再重命名
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

    // 指数退避重试
    let mut delay_ms = RENAME_RETRY_BASE_DELAY_MS;
    for attempt in 1..=RENAME_RETRY_MAX_ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

        // 每次重试前尝试删除目标（可能已解锁）
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

    // 所有重试耗尽 —— 保留 .tmp，让下一次 pull 周期重试
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
