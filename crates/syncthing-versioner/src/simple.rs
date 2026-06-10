//! Simple versioning strategy.
//!
//! Keeps at most `keep` old versions per file in a `.stversions` directory.
//! Versions are tagged with a timestamp in the filename.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;
use tracing::debug;

use crate::{CleanContext, FileVersion, Versioner};
use syncthing_core::SyncthingError;

/// Default name for the version archive directory.
const STVERSIONS_DIR: &str = ".stversions";

pub struct SimpleVersioner {
    versions_dir: PathBuf,
    keep: usize,
}

impl SimpleVersioner {
    pub fn new(folder_path: &Path, keep: usize) -> Self {
        Self {
            versions_dir: folder_path.join(STVERSIONS_DIR),
            keep,
        }
    }

    fn version_path(&self, file_path: &Path, timestamp: &str) -> PathBuf {
        let name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        self.versions_dir.join(format!("{}~{}", name, timestamp))
    }
}

#[async_trait]
impl Versioner for SimpleVersioner {
    async fn archive(&self, file_path: &Path) -> syncthing_core::Result<()> {
        if !file_path.exists() {
            return Ok(());
        }

        fs::create_dir_all(&self.versions_dir)
            .await
            .map_err(SyncthingError::Io)?;

        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let dest = self.version_path(file_path, &timestamp);

        fs::copy(file_path, &dest).await.map_err(|e| {
            SyncthingError::io(format!(
                "Failed to archive {} -> {}: {}",
                file_path.display(),
                dest.display(),
                e
            ))
        })?;

        debug!(
            src = %file_path.display(),
            dest = %dest.display(),
            "Archived file version"
        );

        // Prune excess versions
        self.prune(file_path).await?;

        Ok(())
    }

    async fn get_versions(&self, file_path: &Path) -> syncthing_core::Result<Vec<FileVersion>> {
        let name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let prefix = format!("{}~", name);

        let mut versions = Vec::new();
        let mut entries = match fs::read_dir(&self.versions_dir).await {
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
                versions.push(FileVersion {
                    version_time: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    mod_time: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    size: meta.len(),
                });
            }
        }

        // Sort newest first
        versions.sort_by_key(|b| std::cmp::Reverse(b.version_time));
        Ok(versions)
    }

    async fn restore(
        &self,
        file_path: &Path,
        version_time: SystemTime,
    ) -> syncthing_core::Result<()> {
        let name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let prefix = format!("{}~", name);

        let mut entries = match fs::read_dir(&self.versions_dir).await {
            Ok(e) => e,
            Err(_) => {
                return Err(SyncthingError::internal(format!(
                    "versions dir not found: {}",
                    self.versions_dir.display()
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
            let diff = mtime.duration_since(version_time).unwrap_or_default();
            if diff.as_secs() < 2 {
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

    async fn clean(&self, _ctx: &CleanContext) -> syncthing_core::Result<()> {
        // Simple versioner: clean is handled by archive() prunes.
        // A full CleanContext-based cleanup can be added later.
        Ok(())
    }
}

impl SimpleVersioner {
    /// Remove oldest versions beyond `keep` limit.
    async fn prune(&self, file_path: &Path) -> syncthing_core::Result<()> {
        let mut versions = self.get_versions(file_path).await?;
        if versions.len() <= self.keep {
            return Ok(());
        }

        // Keep the `keep` newest, remove the rest
        versions.sort_by_key(|b| std::cmp::Reverse(b.version_time));
        for old in &versions[self.keep..] {
            let ts = chrono::DateTime::from_timestamp(
                old.version_time
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                0,
            )
            .unwrap_or_default()
            .format("%Y%m%d-%H%M%S")
            .to_string();
            let path = self.version_path(file_path, &ts);
            if path.exists() {
                fs::remove_file(&path).await.map_err(|e| {
                    SyncthingError::io(format!("Failed to prune {}: {}", path.display(), e))
                })?;
                debug!(path = %path.display(), "Pruned old version");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_simple_versioner_archive_and_prune() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        let v = SimpleVersioner::new(&folder, 3);

        let test_file = folder.join("test.txt");
        tokio::fs::write(&test_file, b"v1").await.unwrap();

        // Archive 5 versions (keep=3, oldest 2 should be pruned)
        for i in 0..5u8 {
            tokio::fs::write(&test_file, &[i]).await.unwrap();
            v.archive(&test_file).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        }

        let versions = v.get_versions(&test_file).await.unwrap();
        assert!(
            versions.len() <= 3,
            "Expected <= 3 versions, got {}",
            versions.len()
        );
        assert!(
            versions.len() >= 2,
            "Expected at least 2 versions, got {}",
            versions.len()
        );
    }

    #[tokio::test]
    async fn test_simple_versioner_noop_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let v = SimpleVersioner::new(dir.path(), 5);
        let result = v.archive(Path::new("/nonexistent/file.txt")).await;
        assert!(result.is_ok(), "Archive missing file should be noop");
    }

    #[tokio::test]
    async fn test_simple_versioner_get_empty() {
        let dir = tempfile::tempdir().unwrap();
        let v = SimpleVersioner::new(dir.path(), 5);
        let versions = v
            .get_versions(Path::new("never_archived.txt"))
            .await
            .unwrap();
        assert!(versions.is_empty());
    }
}
