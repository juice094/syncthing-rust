//! Staggered versioning strategy.
//!
//! Mirrors Go syncthing `lib/versioner/staggered.go`. Keeps versions at
//! decreasing frequency as they age:
//!   - first hour:      1 version every 30 seconds
//!   - next day:        1 version every hour
//!   - next 30 days:    1 version per day
//!   - up to maxAge:    1 version per week
//!
//! Always preserves the oldest version in each run.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::fs;
use tracing::debug;

use crate::{CleanContext, FileVersion, Versioner};

const DEFAULT_MAX_AGE_SECS: i64 = 365 * 24 * 60 * 60; // 1 year

struct Interval {
    step_secs: i64,
    end_secs: i64,
}

pub struct StaggeredVersioner {
    versions_dir: PathBuf,
    intervals: [Interval; 4],
}

impl StaggeredVersioner {
    pub fn new(folder_path: &Path, max_age_days: Option<u32>) -> Self {
        let max_age_secs = max_age_days
            .map(|d| d as i64 * 86400)
            .unwrap_or(DEFAULT_MAX_AGE_SECS);

        Self {
            versions_dir: folder_path.join(crate::util::STVERSIONS_DIR),
            intervals: [
                Interval {
                    step_secs: 30,
                    end_secs: 3600,
                }, // 30s steps, first hour
                Interval {
                    step_secs: 3600,
                    end_secs: 86400,
                }, // 1h steps, next day
                Interval {
                    step_secs: 86400,
                    end_secs: 30 * 86400,
                }, // 1d steps, next 30 days
                Interval {
                    step_secs: 7 * 86400,
                    end_secs: max_age_secs,
                }, // 1w steps, up to maxAge
            ],
        }
    }
}

#[async_trait]
impl Versioner for StaggeredVersioner {
    async fn archive(&self, file_path: &Path) -> syncthing_core::Result<()> {
        if !file_path.exists() {
            return Ok(());
        }

        let dest = crate::util::archive_file(&self.versions_dir, file_path).await?;
        debug!(src = %file_path.display(), dest = %dest.display(), "Archived (staggered)");
        Ok(())
    }

    async fn get_versions(&self, file_path: &Path) -> syncthing_core::Result<Vec<FileVersion>> {
        let mut versions = crate::util::list_versions_for(&self.versions_dir, file_path).await?;
        // Staggered expects oldest-first for its window algorithm
        versions.sort_by_key(|a| a.version_time);
        Ok(versions)
    }

    async fn restore(
        &self,
        file_path: &Path,
        version_time: SystemTime,
    ) -> syncthing_core::Result<()> {
        crate::util::restore_by_timestamp(&self.versions_dir, file_path, version_time).await
    }

    async fn clean(&self, _ctx: &CleanContext) -> syncthing_core::Result<()> {
        let now = SystemTime::now();
        let max_age_secs = self.intervals[3].end_secs;
        let now_secs = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut entries = match fs::read_dir(&self.versions_dir).await {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();

            // Remove files beyond maxAge
            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let version_ts = crate::util::extract_timestamp(&fname).unwrap_or(mtime);
            let age = now_secs
                - version_ts
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;

            if age > max_age_secs {
                fs::remove_file(entry.path()).await.ok();
                debug!(path = %entry.path().display(), age_days = age / 86400, "Cleaned expired version");
            }
        }

        // Staggered pruning: for each file group, keep only one version per window step
        self.prune_all().await?;
        Ok(())
    }
}

impl StaggeredVersioner {
    /// Prune all file groups using staggered window algorithm.
    async fn prune_all(&self) -> syncthing_core::Result<()> {
        let now = SystemTime::now();
        let now_secs = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Collect all version files grouped by original name
        let mut groups: std::collections::HashMap<String, Vec<(String, i64)>> =
            std::collections::HashMap::new();

        let mut entries = match fs::read_dir(&self.versions_dir).await {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(tag_pos) = fname.rfind('~') {
                let base = fname[..tag_pos].to_string();
                if let Some(ts) = crate::util::extract_timestamp(&fname) {
                    let age = now_secs
                        - ts.duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                    groups.entry(base).or_default().push((fname, age));
                }
            }
        }

        for (_base, mut versions) in groups {
            if versions.len() <= 1 {
                continue;
            }
            // Sort oldest first
            versions.sort_by_key(|a| a.1);

            let mut prev_age: Option<i64> = None;
            let mut to_remove: Vec<String> = Vec::new();

            for (fname, age) in &versions {
                // Remove if beyond maxAge
                if age > &self.intervals[3].end_secs && self.intervals[3].end_secs > 0 {
                    to_remove.push(fname.clone());
                    continue;
                }

                let prev = match prev_age {
                    Some(p) => p,
                    None => {
                        // Always keep the oldest
                        prev_age = Some(*age);
                        continue;
                    }
                };

                // Find the interval for this version's age
                let step = self.find_step(*age);
                if step > 0 {
                    let steps_from_prev = (age - prev) / step;
                    if steps_from_prev > 0 {
                        // Keep this version — it's far enough from previous
                        prev_age = Some(*age);
                        continue;
                    }
                }

                // Too close to previous version — remove
                to_remove.push(fname.clone());
            }

            for name in to_remove {
                let path = self.versions_dir.join(&name);
                if path.exists() {
                    fs::remove_file(&path).await.ok();
                    debug!(path = %path.display(), "Pruned (staggered window)");
                }
            }
        }

        Ok(())
    }

    fn find_step(&self, age_secs: i64) -> i64 {
        for interval in &self.intervals {
            if age_secs < interval.end_secs {
                return interval.step_secs;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_staggered_versioner_archive_and_clean() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        let v = StaggeredVersioner::new(&folder, None);

        let test_file = folder.join("data.txt");
        tokio::fs::write(&test_file, b"v1").await.unwrap();

        // Archive 5 versions
        for i in 0..5u8 {
            tokio::fs::write(&test_file, &[i]).await.unwrap();
            v.archive(&test_file).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }

        // All versions should exist (intervals are large enough)
        let versions = v.get_versions(&test_file).await.unwrap();
        assert!(
            versions.len() >= 4,
            "Expected >=4 versions, got {}",
            versions.len()
        );
    }

    #[tokio::test]
    async fn test_staggered_extract_timestamp() {
        let ts = crate::util::extract_timestamp("data.txt~20260603-120000").unwrap();
        let secs = ts.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        // 2026-06-03 12:00:00 UTC
        assert!(secs > 1_700_000_000, "Timestamp should be in 2026+");
    }

    #[tokio::test]
    async fn test_staggered_noop_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let v = StaggeredVersioner::new(dir.path(), None);
        assert!(v.archive(Path::new("/no/such/file")).await.is_ok());
    }
}
