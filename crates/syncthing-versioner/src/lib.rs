//! File versioning strategies for syncthing-rust.
//!
//! Mirrors Go syncthing `lib/versioner/`. Each versioner archives
//! replaced or deleted files so they can be restored later.

pub mod simple;
pub mod staggered;
pub(crate) mod util;

use async_trait::async_trait;
use std::path::Path;
use std::time::SystemTime;
use syncthing_core::types::VersioningConfig;

/// A single archived version of a file.
#[derive(Debug, Clone)]
pub struct FileVersion {
    pub version_time: SystemTime,
    pub mod_time: SystemTime,
    pub size: u64,
}

/// Context for a version cleanup pass.
#[derive(Debug, Clone)]
pub struct CleanContext {
    pub now: SystemTime,
}

/// Trait for file versioning strategies.
#[async_trait]
pub trait Versioner: Send + Sync {
    /// Archive the file at `file_path` before it is overwritten or deleted.
    async fn archive(&self, file_path: &Path) -> syncthing_core::Result<()>;

    /// List all versions of the given file.
    async fn get_versions(&self, file_path: &Path) -> syncthing_core::Result<Vec<FileVersion>>;

    /// Restore a specific version.
    async fn restore(
        &self,
        file_path: &Path,
        version_time: SystemTime,
    ) -> syncthing_core::Result<()>;

    /// Remove expired versions according to configured policy.
    async fn clean(&self, ctx: &CleanContext) -> syncthing_core::Result<()>;
}

/// Factory: construct the appropriate versioner from folder configuration.
pub fn create_versioner(cfg: &VersioningConfig, folder_path: &Path) -> Option<Box<dyn Versioner>> {
    match cfg {
        VersioningConfig::None => None,
        VersioningConfig::Simple { params } => {
            let keep = params
                .get("keep")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(5);
            Some(Box::new(simple::SimpleVersioner::new(folder_path, keep)))
        }
        VersioningConfig::Staggered { params } => {
            let max_age = params.get("maxAge").and_then(|v| v.parse::<u32>().ok());
            Some(Box::new(staggered::StaggeredVersioner::new(
                folder_path,
                max_age,
            )))
        }
        VersioningConfig::External { .. } => {
            tracing::warn!("External versioning not yet implemented");
            None
        }
    }
}
