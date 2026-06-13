//! Shared helpers for versioner implementations.
//!
//! Extracts common logic between `simple` and `staggered` strategies to avoid
//! duplication of filename handling, directory listing, and timestamp parsing.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::fs;

use crate::FileVersion;
use syncthing_core::SyncthingError;

/// Default name for the version archive directory.
pub const STVERSIONS_DIR: &str = ".stversions";

/// Filename timestamp format used by all versioners.
pub const TIMESTAMP_FORMAT: &str = "%Y%m%d-%H%M%S";

/// Build the archive path for a file at a given timestamp.
pub fn version_path(versions_dir: &Path, file_path: &Path, timestamp: &str) -> PathBuf {
    let name = file_name_or_unknown(file_path);
    versions_dir.join(format!("{}~{}", name, timestamp))
}

/// Extract the base file name from a path, falling back to "unknown".
pub fn file_name_or_unknown(file_path: &Path) -> String {
    file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Extract version timestamp from filename (format: `name~YYYYMMDD-HHMMSS`).
pub fn extract_timestamp(filename: &str) -> Option<SystemTime> {
    let tag = filename.rsplit('~').next()?;
    let parsed = chrono::NaiveDateTime::parse_from_str(tag, TIMESTAMP_FORMAT).ok()?;
    let dt = parsed.and_utc();
    Some(
        std::time::UNIX_EPOCH
            + Duration::from_secs(dt.timestamp() as u64)
            + Duration::from_nanos(dt.timestamp_subsec_nanos() as u64),
    )
}

/// Build the filename prefix used to identify versions of a given file.
pub fn version_prefix(file_path: &Path) -> String {
    format!("{}~", file_name_or_unknown(file_path))
}

/// List all archived versions of `file_path` in `versions_dir`.
///
/// Returns `FileVersion` entries sorted newest-first. If `versions_dir` does not
/// exist, returns an empty vector.
pub async fn list_versions_for(
    versions_dir: &Path,
    file_path: &Path,
) -> syncthing_core::Result<Vec<FileVersion>> {
    let prefix = version_prefix(file_path);
    let mut versions = Vec::new();

    let mut entries = match fs::read_dir(versions_dir).await {
        Ok(e) => e,
        Err(_) => return Ok(versions),
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let fname = entry.file_name();
        let fname = fname.to_string_lossy();
        if !fname.starts_with(&prefix) {
            continue;
        }
        if let Ok(meta) = entry.metadata().await {
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            versions.push(FileVersion {
                version_time: extract_timestamp(&fname).unwrap_or(mtime),
                mod_time: mtime,
                size: meta.len(),
            });
        }
    }

    // Sort newest first
    versions.sort_by_key(|b| std::cmp::Reverse(b.version_time));
    Ok(versions)
}

/// Restore the version of `file_path` whose archive time is closest to
/// `version_time` (within a 2-second tolerance).
pub async fn restore_by_timestamp(
    versions_dir: &Path,
    file_path: &Path,
    version_time: SystemTime,
) -> syncthing_core::Result<()> {
    let prefix = version_prefix(file_path);

    let mut entries = match fs::read_dir(versions_dir).await {
        Ok(e) => e,
        Err(_) => {
            return Err(SyncthingError::internal(format!(
                "versions dir not found: {}",
                versions_dir.display()
            )))
        }
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let fname = entry.file_name();
        let fname = fname.to_string_lossy();
        if !fname.starts_with(&prefix) {
            continue;
        }
        let meta = entry.metadata().await.map_err(SyncthingError::Io)?;
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let version_ts = extract_timestamp(&fname).unwrap_or(mtime);
        let diff = version_ts
            .duration_since(version_time)
            .unwrap_or_else(|_| version_time.duration_since(version_ts).unwrap_or_default())
            .as_secs();
        if diff < 2 {
            fs::copy(entry.path(), file_path).await.map_err(|e| {
                SyncthingError::io(format!(
                    "Failed to restore {} -> {}: {}",
                    entry.path().display(),
                    file_path.display(),
                    e
                ))
            })?;
            return Ok(());
        }
    }

    Err(SyncthingError::internal(format!(
        "version not found for {}",
        file_path.display()
    )))
}

/// Copy `file_path` into the archive directory with a timestamped name.
pub async fn archive_file(
    versions_dir: &Path,
    file_path: &Path,
) -> syncthing_core::Result<PathBuf> {
    fs::create_dir_all(versions_dir)
        .await
        .map_err(SyncthingError::Io)?;

    let timestamp = chrono::Utc::now().format(TIMESTAMP_FORMAT).to_string();
    let dest = version_path(versions_dir, file_path, &timestamp);

    fs::copy(file_path, &dest).await.map_err(|e| {
        SyncthingError::io(format!(
            "Failed to archive {} -> {}: {}",
            file_path.display(),
            dest.display(),
            e
        ))
    })?;

    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_timestamp() {
        let ts = extract_timestamp("data.txt~20260603-120000").unwrap();
        let secs = ts.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert!(secs > 1_700_000_000, "Timestamp should be in 2026+");
    }

    #[test]
    fn test_extract_timestamp_invalid() {
        assert!(extract_timestamp("data.txt~invalid").is_none());
        assert!(extract_timestamp("data.txt").is_none());
    }
}
