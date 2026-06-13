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

pub struct SimpleVersioner {
    versions_dir: PathBuf,
    keep: usize,
}

impl SimpleVersioner {
    pub fn new(folder_path: &Path, keep: usize) -> Self {
        Self {
            versions_dir: folder_path.join(crate::util::STVERSIONS_DIR),
            keep,
        }
    }
}

#[async_trait]
impl Versioner for SimpleVersioner {
    async fn archive(&self, file_path: &Path) -> syncthing_core::Result<()> {
        if !file_path.exists() {
            return Ok(());
        }

        let dest = crate::util::archive_file(&self.versions_dir, file_path).await?;
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
        crate::util::list_versions_for(&self.versions_dir, file_path).await
    }

    async fn restore(
        &self,
        file_path: &Path,
        version_time: SystemTime,
    ) -> syncthing_core::Result<()> {
        crate::util::restore_by_timestamp(&self.versions_dir, file_path, version_time).await
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
        let versions = self.get_versions(file_path).await?;
        if versions.len() <= self.keep {
            return Ok(());
        }

        // Keep the `keep` newest, remove the rest
        for old in &versions[self.keep..] {
            let ts = chrono::DateTime::from_timestamp(
                old.version_time
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                0,
            )
            .unwrap_or_default()
            .format(crate::util::TIMESTAMP_FORMAT)
            .to_string();
            let path = crate::util::version_path(&self.versions_dir, file_path, &ts);
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
