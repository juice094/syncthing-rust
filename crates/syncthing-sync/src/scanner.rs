//! Folder scanner.
//!
//! Detects local folder changes through periodic full scans and watcher-
//! driven incremental scans.
//!
//! See "docs/design/ADR-20261105-scanner-incremental-paths.md".

mod rename;
pub(crate) use rename::detect_and_reorder_renames;

use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{EventPublisher, SyncEvent};
use crate::ignore::IgnoreMatcher;
use crate::puller::temp_path_for;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use syncthing_core::types::{BlockInfo, FileInfo, FileInfoBase, FileType, Folder, Vector};
use tokio::fs;
use tracing::{debug, error, info, trace};

/// Block size (128 KiB, matching the Syncthing default).
const DEFAULT_BLOCK_SIZE: i32 = syncthing_core::constants::DEFAULT_BLOCK_SIZE;

/// Names always excluded by the scanner (aligned with Go Syncthing). These
/// take effect before `.stignore` is loaded, preventing sync metadata from
/// being indexed and synchronized.
const DEFAULT_IGNORED_NAMES: &[&str] = &[
    ".stfolder",
    ".stversions",
    ".sttrash",
    ".stignore",
    "config.json",
    "cert.pem",
    "key.pem",
    "db",
    "logs",
];

/// Suffix patterns always excluded by the scanner (temporary files).
const DEFAULT_IGNORED_SUFFIXES: &[&str] = &[".syncthing.tmp", "~syncthing~"];

/// Returns `true` when any component of the relative path is default-excluded
/// metadata (`.stversions`/`.sttrash`/etc.). Every watcher incremental path
/// (`scan_changed_file` / `mark_deleted_subtree` / sub-scan root) must pass
/// this check, otherwise metadata enters the index and gets synchronized,
/// forming an archive↔sync feedback loop. See
/// "docs/design/ADR-20261105-scanner-incremental-paths.md".
fn is_default_ignored_path(relative_path: &str) -> bool {
    relative_path
        .split(['/', '\\'])
        .any(|c| DEFAULT_IGNORED_NAMES.contains(&c))
}

/// Returns `true` when the relative path itself or any ancestor directory
/// matches an ignore rule.
///
/// `IgnoreMatcher::matches` only applies directory rules (`dir/`) when
/// `is_dir = true`, but watcher incremental paths are usually **file** paths;
/// checking only the file itself lets new/modified files inside ignored
/// directories be indexed and synchronized (historical defect, see
/// `docs/KNOWN_ISSUES.md` §23). Callers therefore pass `is_dir` semantically
/// (`false` for file changes, `true` for sub-scan roots) and ancestors are
/// always checked as directories. See
/// "docs/design/ADR-20261105-scanner-incremental-paths.md".
fn is_ignored_with_ancestors(matcher: &IgnoreMatcher, relative_path: &str, is_dir: bool) -> bool {
    let normalized = relative_path.replace('\\', "/");
    if matcher.matches(&normalized, is_dir) {
        return true;
    }

    let mut prefix = String::new();
    let mut segments = normalized.split('/').peekable();
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            // The last segment is the path itself, already checked above.
            break;
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        if matcher.matches(&prefix, true) {
            return true;
        }
    }
    false
}

/// Scans a folder's contents and produces `FileInfo` updates.
pub struct Scanner {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    /// Local version-vector counter ID (derived from the real device ID;
    /// hardcoding it would misclassify concurrent offline edits as linear
    /// history, silently letting the deleter overwrite the peer's changes).
    local_counter_id: u64,
}

impl Scanner {
    /// Creates a new scanner.
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher, local_counter_id: u64) -> Self {
        Self {
            db,
            events,
            local_counter_id,
        }
    }

    /// Scans a whole folder and returns the changed file entries.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when the folder path is missing/not a directory
    /// or a filesystem/database operation fails.
    pub async fn scan_folder(&self, folder: &Folder) -> Result<Vec<FileInfo>> {
        self.scan_folder_at(folder, None).await
    }

    /// Scans a subdirectory of a folder (watcher incremental path).
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when a filesystem/database operation fails.
    pub async fn scan_folder_sub(&self, folder: &Folder, sub: &str) -> Result<Vec<FileInfo>> {
        self.scan_folder_at(folder, Some(sub)).await
    }

    async fn scan_folder_at(&self, folder: &Folder, sub: Option<&str>) -> Result<Vec<FileInfo>> {
        // Reject sub-scan roots inside default-excluded metadata directories
        // at the entry point (per-entry name filtering cannot catch them, see
        // the module-level ADR pointer).
        if let Some(s) = sub {
            if is_default_ignored_path(s) {
                debug!(folder_id = %folder.id, sub = %s, "Sub-scan root is default-ignored metadata, skipping");
                return Ok(Vec::new());
            }
        }

        let path = Path::new(&folder.path);
        let scan_root = match sub {
            Some(s) => path.join(s),
            None => path.to_path_buf(),
        };

        info!(folder_id = %folder.id, path = %scan_root.display(), "Starting folder scan");

        if !scan_root.exists() {
            return Err(SyncError::scan(
                folder.id.clone(),
                format!("Path does not exist: {}", scan_root.display()),
            ));
        }

        if !scan_root.is_dir() {
            return Err(SyncError::scan(
                folder.id.clone(),
                format!("Path is not a directory: {}", scan_root.display()),
            ));
        }

        let mut changed_files = Vec::new();
        let mut visited_paths = std::collections::HashSet::new();

        let ignore_path = path.join(".stignore");
        let mut matcher = IgnoreMatcher::load(&ignore_path);
        let file_rule_count = matcher.len();

        for pattern in &folder.ignore_patterns {
            matcher.add_line(pattern);
        }

        info!(
            folder_id = %folder.id,
            stignore_path = %ignore_path.display(),
            file_rules = file_rule_count,
            config_rules = folder.ignore_patterns.len(),
            total_rules = matcher.len(),
            "Loaded ignore patterns"
        );

        // Sub-scan roots must pass ignore rules too (§23, see the
        // module-level ADR pointer).
        if let Some(s) = sub {
            if is_ignored_with_ancestors(&matcher, s, true) {
                debug!(
                    folder_id = %folder.id,
                    sub = %s,
                    "Sub-scan root is matched by stignore, skipping"
                );
                return Ok(Vec::new());
            }
        }

        // Index names must stay relative to the folder root even when the
        // scan root is a subdirectory (W16; see the module-level ADR pointer).
        let relative_prefix = match sub {
            Some(s) => Path::new(s),
            None => Path::new(""),
        };
        match self
            .scan_directory(
                &folder.id,
                path,
                &scan_root,
                relative_prefix,
                &mut visited_paths,
                &matcher,
            )
            .await
        {
            Ok(files) => {
                if sub.is_none() {
                    let db_files = self.db.get_folder_files(&folder.id).await?;
                    for db_file in db_files {
                        let full_path = path.join(&db_file.name);
                        if !full_path.exists() && !db_file.is_deleted() {
                            let temp_path = temp_path_for(&full_path);
                            if temp_path.exists() {
                                debug!(file = %db_file.name, "File is being downloaded, skipping deleted check");
                                continue;
                            }
                            debug!(file = %db_file.name, "File was deleted");
                            let mut deleted_info = db_file.clone();
                            deleted_info.deleted = Some(true);
                            // Blocks are kept here for rename detection and
                            // cleared below (see the module-level ADR pointer).
                            deleted_info.size = 0;
                            deleted_info.sequence = self.db.increment_sequence(&folder.id).await?;
                            deleted_info.version.increment(self.local_counter_id);
                            changed_files.push(deleted_info);
                        }
                    }
                }

                let scanned_count = files.len();
                let mut new_count = 0u64;
                let mut modified_count = 0u64;
                for file_info in files {
                    match self.db.get_file(&folder.id, &file_info.name).await? {
                        Some(existing) => {
                            if Self::has_file_changed(&existing, &file_info) {
                                debug!(file = %file_info.name, "File was modified");
                                modified_count += 1;
                                let mut new_info = file_info;
                                new_info.sequence = self.db.increment_sequence(&folder.id).await?;
                                new_info.version = existing.version.clone();
                                new_info.version.increment(self.local_counter_id);
                                new_info.base_version = existing.base_version.clone();
                                changed_files.push(new_info);
                            }
                        }
                        None => {
                            debug!(file = %file_info.name, "New file found");
                            new_count += 1;
                            let mut new_info = file_info;
                            new_info.sequence = self.db.increment_sequence(&folder.id).await?;
                            new_info.version = Vector::new().with_counter(1, 1);
                            changed_files.push(new_info);
                        }
                    }
                }
                info!(
                    folder_id = %folder.id,
                    scanned = scanned_count,
                    new = new_count,
                    modified = modified_count,
                    changed = changed_files.len(),
                    "Scan file comparison complete"
                );

                if sub.is_none() {
                    changed_files = detect_and_reorder_renames(changed_files);
                }

                // Deleted entries must carry empty block lists (BEP requirement).
                for file in &mut changed_files {
                    if file.is_deleted() {
                        file.blocks.clear();
                    }
                }
            }
            Err(e) => {
                error!(folder_id = %folder.id, error = %e, "Scan failed");
                self.events.publish(SyncEvent::FolderScanFailed {
                    folder: folder.id.clone(),
                    error: e.to_string(),
                });
                return Err(e);
            }
        }

        for file in &changed_files {
            let file_clone: syncthing_core::types::FileInfo = file.clone();
            self.db.update_file(&folder.id, file_clone).await?;
        }

        info!(
            folder_id = %folder.id,
            files_changed = changed_files.len(),
            "Folder scan completed"
        );

        self.events.publish(SyncEvent::FolderScanCompleted {
            folder: folder.id.clone(),
            files_changed: changed_files.len(),
        });

        Ok(changed_files)
    }

    /// Recursively scans a directory.
    ///
    /// `relative_prefix` accumulates per recursion level, completely avoiding
    /// the platform differences of `Path::strip_prefix` on Windows UNC /
    /// mixed-separator paths.
    #[async_recursion::async_recursion]
    async fn scan_directory(
        &self,
        folder_id: &str,
        _base_path: &Path,
        current_path: &Path,
        relative_prefix: &Path,
        visited: &mut std::collections::HashSet<std::path::PathBuf>,
        matcher: &IgnoreMatcher,
    ) -> Result<Vec<FileInfo>> {
        let mut files = Vec::new();

        let entries = std::fs::read_dir(current_path).map_err(|e| {
            SyncError::scan(
                folder_id.to_string(),
                format!("Failed to read directory: {}", e),
            )
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                SyncError::scan(
                    folder_id.to_string(),
                    format!("Failed to read entry: {}", e),
                )
            })?;
            let path = entry.path();

            let file_name_os = match path.file_name() {
                Some(name) => name,
                None => continue,
            };
            let name = file_name_os.to_string_lossy();

            if DEFAULT_IGNORED_NAMES.iter().any(|&ignored| name == ignored) {
                trace!(path = %path.display(), "Skipping default ignored syncthing metadata");
                continue;
            }
            if DEFAULT_IGNORED_SUFFIXES
                .iter()
                .any(|&suffix| name.ends_with(suffix))
            {
                trace!(path = %path.display(), "Skipping default ignored suffix");
                continue;
            }

            if name.starts_with('.') || name.starts_with("~") || name.ends_with(".tmp") {
                trace!(path = %path.display(), "Skipping hidden/temp file");
                continue;
            }
            if name.contains(".sync-conflict-") {
                trace!(path = %path.display(), "Skipping conflict file");
                continue;
            }

            let metadata = entry.metadata().map_err(|e| {
                SyncError::scan(
                    folder_id.to_string(),
                    format!("Failed to get metadata: {}", e),
                )
            })?;

            let relative_path = if relative_prefix.as_os_str().is_empty() {
                name.replace('\\', "/")
            } else {
                format!(
                    "{}/{}",
                    relative_prefix.to_string_lossy().replace('\\', "/"),
                    name.replace('\\', "/")
                )
            };

            let is_dir = metadata.is_dir();
            if matcher.matches(&relative_path, is_dir) {
                trace!(path = %relative_path, "Ignoring path via .stignore");
                continue;
            }

            if visited.contains(&path) {
                continue;
            }
            visited.insert(path.clone());

            if is_dir {
                let mut next_prefix = relative_prefix.to_path_buf();
                next_prefix.push(file_name_os);
                let sub_files = self
                    .scan_directory(folder_id, _base_path, &path, &next_prefix, visited, matcher)
                    .await?;
                files.extend(sub_files);

                let modified = metadata
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let modified_secs = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let modified_nanos = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as i32;

                files.push(FileInfo {
                    name: relative_path.clone(),
                    file_type: FileType::Directory,
                    size: 0,
                    permissions: 0o755,
                    modified_s: modified_secs,
                    modified_ns: modified_nanos,
                    version: Vector::new(),
                    sequence: 0,
                    block_size: 0,
                    blocks: vec![],
                    symlink_target: None,
                    deleted: None,
                    modified_by: None,
                    blocks_hash: None,
                    no_permissions: None,
                    base_version: Some(FileInfoBase {
                        size: 0,
                        modified_s: modified_secs,
                        modified_ns: modified_nanos,
                        blocks_hash: None,
                        content_hash: None,
                    }),
                });
            } else if metadata.is_file() {
                let file_info = self
                    .scan_file(&path, &relative_path, &metadata, folder_id)
                    .await?;
                files.push(file_info);
            } else if metadata.is_symlink() {
                let target = fs::read_link(&path).await.map_err(|e| {
                    SyncError::scan(
                        folder_id.to_string(),
                        format!("Failed to read symlink: {}", e),
                    )
                })?;

                let modified = metadata
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let modified_secs = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;

                files.push(FileInfo {
                    name: relative_path,
                    file_type: FileType::Symlink,
                    size: 0,
                    permissions: 0o755,
                    modified_s: modified_secs,
                    modified_ns: 0,
                    version: Vector::new(),
                    sequence: 0,
                    block_size: 0,
                    blocks: vec![],
                    symlink_target: Some(target.to_string_lossy().to_string()),
                    deleted: None,
                    modified_by: None,
                    blocks_hash: None,
                    no_permissions: None,
                    base_version: Some(FileInfoBase {
                        size: 0,
                        modified_s: modified_secs,
                        modified_ns: 0,
                        blocks_hash: None,
                        content_hash: None,
                    }),
                });
            }
        }

        Ok(files)
    }

    /// Scans a single file: computes block hashes and the whole-content hash.
    async fn scan_file(
        &self,
        path: &Path,
        relative_path: &str,
        metadata: &std::fs::Metadata,
        folder_id: &str,
    ) -> Result<FileInfo> {
        let size = metadata.len() as i64;
        let modified = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let modified_secs = modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let modified_nanos = modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as i32;

        let permissions = 0o644;

        let mut blocks = Vec::new();
        let mut content_hasher = Sha256::new();
        if size > 0 {
            let block_size = DEFAULT_BLOCK_SIZE;
            let num_blocks = ((size + block_size as i64 - 1) / block_size as i64) as usize;

            let file = tokio::fs::File::open(path).await.map_err(|e| {
                SyncError::scan(folder_id.to_string(), format!("Failed to open file: {}", e))
            })?;

            let mut reader = tokio::io::BufReader::new(file);
            let mut buffer = vec![0u8; block_size as usize];
            let mut offset = 0i64;

            for i in 0..num_blocks {
                let bytes_read = tokio::io::AsyncReadExt::read(&mut reader, &mut buffer)
                    .await
                    .map_err(|e| {
                        SyncError::scan(
                            folder_id.to_string(),
                            format!("Failed to read file: {}", e),
                        )
                    })?;

                if bytes_read == 0 {
                    break;
                }

                let mut hasher = Sha256::new();
                hasher.update(&buffer[..bytes_read]);
                let hash = hasher.finalize().to_vec();
                content_hasher.update(&buffer[..bytes_read]);

                blocks.push(BlockInfo {
                    size: bytes_read as i32,
                    hash,
                    offset,
                });

                offset += bytes_read as i64;

                if i % 100 == 0 {
                    trace!(file = %relative_path, block = i, "Scanning block");
                }
            }
        } else {
            // Zero-byte files must still carry one empty block for BEP
            // compatibility (see the module-level ADR pointer).
            let empty_hash = Sha256::new().finalize().to_vec();
            blocks.push(BlockInfo {
                size: 0,
                hash: empty_hash,
                offset: 0,
            });
        }

        let content_hash = content_hasher.finalize().to_vec();
        Ok(FileInfo {
            name: relative_path.to_string(),
            file_type: FileType::File,
            size,
            permissions,
            modified_s: modified_secs,
            modified_ns: modified_nanos,
            version: Vector::new(),
            sequence: 0,
            block_size: DEFAULT_BLOCK_SIZE,
            blocks,
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: Some(FileInfoBase {
                size,
                modified_s: modified_secs,
                modified_ns: modified_nanos,
                blocks_hash: None,
                content_hash: Some(content_hash),
            }),
        })
    }

    /// Returns `true` when the file content changed between the old and new entries.
    ///
    /// Content hashes take precedence: identical block lists mean "unchanged"
    /// regardless of mtime/permission noise, preventing infinite
    /// version ping-pong across platforms. Purely metadata-only changes
    /// (mtime/permissions) are currently not propagated; see
    /// "docs/design/ADR-20261105-scanner-change-detection.md".
    fn has_file_changed(old: &FileInfo, new: &FileInfo) -> bool {
        if old.blocks.len() == new.blocks.len()
            && old
                .blocks
                .iter()
                .zip(new.blocks.iter())
                .all(|(o, n)| o.hash == n.hash)
        {
            return false;
        }

        if old.size != new.size {
            return true;
        }

        if old.modified_s != new.modified_s || old.modified_ns != new.modified_ns {
            return true;
        }

        if old.permissions != new.permissions {
            return true;
        }

        true
    }

    /// Scans a single relative path (incremental scan).
    ///
    /// Returns the changed `FileInfo`, or `None` when the file is unchanged,
    /// missing, excluded, or ignored.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when metadata cannot be read or a database
    /// operation fails.
    pub async fn scan_changed_file(
        &self,
        folder: &Folder,
        relative_path: &str,
    ) -> Result<Option<FileInfo>> {
        if is_default_ignored_path(relative_path) {
            trace!(path = %relative_path, "Skipping default-ignored metadata path");
            return Ok(None);
        }

        // Config ignore rules must apply to incremental paths as well (§23,
        // see the module-level ADR pointer).
        let ignore_path = Path::new(&folder.path).join(".stignore");
        let mut matcher = IgnoreMatcher::load(&ignore_path);
        for pattern in &folder.ignore_patterns {
            matcher.add_line(pattern);
        }
        if is_ignored_with_ancestors(&matcher, relative_path, false) {
            trace!(path = %relative_path, "Skipping stignore-matched path (incremental)");
            return Ok(None);
        }

        let base_path = Path::new(&folder.path);
        let full_path = base_path.join(relative_path);

        let metadata = match tokio::fs::metadata(&full_path).await {
            Ok(m) if m.is_file() => m,
            Ok(_) => return Ok(None),
            Err(e) => {
                return Err(SyncError::scan(
                    folder.id.clone(),
                    format!("Failed to get metadata for {}: {}", relative_path, e),
                ))
            }
        };

        let file_info = self
            .scan_file(&full_path, relative_path, &metadata, &folder.id)
            .await?;
        match self.db.get_file(&folder.id, relative_path).await? {
            Some(existing) if !Self::has_file_changed(&existing, &file_info) => Ok(None),
            Some(existing) => {
                let mut new_info = file_info;
                new_info.sequence = self.db.increment_sequence(&folder.id).await?;
                new_info.version = existing.version.clone();
                new_info.version.increment(self.local_counter_id);
                new_info.base_version = existing.base_version.clone();
                Ok(Some(new_info))
            }
            None => {
                let mut new_info = file_info;
                new_info.sequence = self.db.increment_sequence(&folder.id).await?;
                new_info.version = Vector::new().with_counter(1, 1);
                Ok(Some(new_info))
            }
        }
    }

    /// Marks a single relative path as deleted (when known to the DB and not
    /// already deleted).
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when the database lookup fails.
    pub async fn mark_deleted(
        &self,
        folder: &Folder,
        relative_path: &str,
    ) -> Result<Option<FileInfo>> {
        match self.db.get_file(&folder.id, relative_path).await? {
            Some(db_file) if !db_file.is_deleted() => {
                let mut deleted_info = db_file.clone();
                deleted_info.deleted = Some(true);
                deleted_info.size = 0;
                deleted_info.blocks.clear();
                deleted_info.sequence = self.db.increment_sequence(&folder.id).await?;
                deleted_info.version.increment(self.local_counter_id);
                Ok(Some(deleted_info))
            }
            _ => Ok(None),
        }
    }

    /// Marks DB files under a subtree as deleted when they no longer exist on
    /// disk (used by incremental scans to discover subtree deletions).
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when the database lookup fails.
    pub async fn mark_deleted_subtree(
        &self,
        folder: &Folder,
        relative_prefix: &str,
    ) -> Result<Vec<FileInfo>> {
        // Deletions under metadata directories are never broadcast (they were
        // never indexed; propagating them would delete `.sttrash` on peers).
        if is_default_ignored_path(relative_prefix) {
            return Ok(Vec::new());
        }

        let prefix_with_slash = if relative_prefix.ends_with('/') {
            relative_prefix.to_string()
        } else {
            format!("{}/", relative_prefix)
        };
        let base_path = Path::new(&folder.path);
        let db_files = self.db.get_folder_files(&folder.id).await?;
        let mut deleted = Vec::new();
        for db_file in db_files {
            if db_file.is_deleted() {
                continue;
            }
            if db_file.name == relative_prefix || db_file.name.starts_with(&prefix_with_slash) {
                // Only mark files that actually vanished from disk; blind
                // deletion would move the whole subtree into `.sttrash`
                // (W16 root cause #2, see the module-level ADR pointer).
                let full_path = base_path.join(&db_file.name);
                if full_path.exists() {
                    continue;
                }
                let mut deleted_info = db_file.clone();
                deleted_info.deleted = Some(true);
                deleted_info.size = 0;
                deleted_info.blocks.clear();
                deleted_info.sequence = self.db.increment_sequence(&folder.id).await?;
                deleted_info.version.increment(self.local_counter_id);
                deleted.push(deleted_info);
            }
        }
        Ok(deleted)
    }

    /// Quick scan (mtime-only check against the DB).
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] when the database lookup fails.
    pub async fn quick_scan(&self, folder: &Folder) -> Result<Vec<FileInfo>> {
        debug!(folder_id = %folder.id, "Starting quick scan");

        let db_files: Vec<syncthing_core::types::FileInfo> =
            self.db.get_folder_files(&folder.id).await?;
        let mut changed = Vec::new();
        let base_path = Path::new(&folder.path);

        for db_file in db_files {
            if db_file.is_deleted() {
                continue;
            }

            let full_path = base_path.join(&db_file.name);
            match fs::metadata(&full_path).await {
                Ok(metadata) => {
                    let modified = metadata
                        .modified()
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    let modified_secs = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    let modified_nanos = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos() as i32;

                    if db_file.modified_s != modified_secs || db_file.modified_ns != modified_nanos
                    {
                        changed.push(db_file);
                    }
                }
                Err(_) => {
                    let temp_path = temp_path_for(&full_path);
                    if temp_path.exists() {
                        debug!(file = %db_file.name, "File is being downloaded, skipping deleted check");
                        continue;
                    }
                    if !db_file.is_deleted() {
                        changed.push(db_file);
                    }
                }
            }
        }

        debug!(folder_id = %folder.id, changed_count = changed.len(), "Quick scan completed");
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryDatabase;

    #[tokio::test]
    async fn test_scan_empty_folder() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let scanner = Scanner::new(db, events, 1);

        // 创建临时目录
        let temp_dir = tempfile::tempdir().unwrap();
        let folder = Folder::new("test", temp_dir.path().to_str().unwrap());

        let result = scanner.scan_folder(&folder).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_scan_zero_byte_file_has_one_empty_block() {
        use sha2::{Digest, Sha256};

        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let scanner = Scanner::new(db, events, 1);

        let temp_dir = tempfile::tempdir().unwrap();
        let empty_path = temp_dir.path().join("empty.txt");
        tokio::fs::write(&empty_path, b"").await.unwrap();

        let folder = Folder::new("test", temp_dir.path().to_str().unwrap());
        let result = scanner.scan_folder(&folder).await.unwrap();

        assert_eq!(result.len(), 1);
        let file = &result[0];
        assert_eq!(file.name, "empty.txt");
        assert_eq!(file.size, 0);
        assert_eq!(file.blocks.len(), 1);
        assert_eq!(file.blocks[0].size, 0);
        assert_eq!(file.blocks[0].offset, 0);
        assert_eq!(file.blocks[0].hash, Sha256::new().finalize().to_vec());
    }

    #[tokio::test]
    async fn test_sub_scan_of_default_ignored_dir_returns_empty() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let scanner = Scanner::new(db, events, 1);

        let temp_dir = tempfile::tempdir().unwrap();
        let stversions = temp_dir.path().join(".stversions");
        tokio::fs::create_dir(&stversions).await.unwrap();
        tokio::fs::write(stversions.join("note.md~20260721-010000"), b"old")
            .await
            .unwrap();

        let folder = Folder::new("test", temp_dir.path().to_str().unwrap());
        // 全量扫描本就不应包含 .stversions
        assert!(scanner.scan_folder(&folder).await.unwrap().is_empty());
        // watcher 增量路径：子扫描根就是 .stversions，也不应索引任何内容
        let files = scanner
            .scan_folder_sub(&folder, ".stversions")
            .await
            .unwrap();
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn test_stignore_dir_rule_applies_to_watcher_paths() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let scanner = Scanner::new(db, events, 1);

        let temp_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(temp_dir.path().join(".stignore"), "ignored/\n")
            .await
            .unwrap();
        let ignored = temp_dir.path().join("ignored");
        tokio::fs::create_dir(&ignored).await.unwrap();
        tokio::fs::write(ignored.join("secret.txt"), b"secret")
            .await
            .unwrap();
        let kept = temp_dir.path().join("kept");
        tokio::fs::create_dir(&kept).await.unwrap();
        tokio::fs::write(kept.join("public.txt"), b"public")
            .await
            .unwrap();

        let folder = Folder::new("test", temp_dir.path().to_str().unwrap());

        // 全量扫描：kept/public.txt 必须在，被忽略目录下的任何条目都不得出现
        let files = scanner.scan_folder(&folder).await.unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(
            names.contains(&"kept/public.txt"),
            "kept/public.txt 应被索引: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.starts_with("ignored")),
            "被忽略目录不应出现在索引中: {:?}",
            names
        );

        // watcher 增量路径同样要遵守 `.stignore`（回归 §23）：
        // 目录规则只在 is_dir=true 时命中，因此必须逐级检查祖先目录
        assert!(scanner
            .scan_changed_file(&folder, "ignored/secret.txt")
            .await
            .unwrap()
            .is_none());
        // 子扫描根落在被忽略目录时返回空
        assert!(scanner
            .scan_folder_sub(&folder, "ignored")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn test_changed_file_and_delete_marking_skip_metadata_paths() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let scanner = Scanner::new(db, events, 1);

        let temp_dir = tempfile::tempdir().unwrap();
        let trash = temp_dir.path().join(".sttrash").join("20260721");
        tokio::fs::create_dir_all(&trash).await.unwrap();
        tokio::fs::write(trash.join("note.md"), b"old")
            .await
            .unwrap();

        let folder = Folder::new("test", temp_dir.path().to_str().unwrap());
        // 单文件增量扫描：.sttrash 内文件不进入索引
        assert!(scanner
            .scan_changed_file(&folder, ".sttrash/20260721/note.md")
            .await
            .unwrap()
            .is_none());
        // 删除标记：元数据路径不广播删除
        assert!(scanner
            .mark_deleted_subtree(&folder, ".sttrash/20260721/note.md")
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_detect_rename_reorders_files() {
        let old_file = FileInfo {
            name: "old_name.txt".to_string(),
            file_type: FileType::File,
            size: 0,
            permissions: 0,
            modified_s: 0,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 1,
            block_size: 0,
            blocks: vec![BlockInfo {
                size: 10,
                hash: vec![1, 2, 3],
                offset: 0,
            }],
            symlink_target: None,
            deleted: Some(true),
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        };

        let new_file = FileInfo {
            name: "new_name.txt".to_string(),
            file_type: FileType::File,
            size: 10,
            permissions: 0,
            modified_s: 0,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 2,
            block_size: 0,
            blocks: vec![BlockInfo {
                size: 10,
                hash: vec![1, 2, 3],
                offset: 0,
            }],
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        };

        let unchanged = FileInfo {
            name: "unchanged.txt".to_string(),
            file_type: FileType::File,
            size: 5,
            permissions: 0,
            modified_s: 0,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 3,
            block_size: 0,
            blocks: vec![BlockInfo {
                size: 5,
                hash: vec![4, 5, 6],
                offset: 0,
            }],
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        };

        let input = vec![old_file, unchanged, new_file];
        let result = detect_and_reorder_renames(input);

        // 新文件应排在最前面
        assert_eq!(result[0].name, "new_name.txt");
        // 其余保持原顺序
        assert_eq!(result[1].name, "old_name.txt");
        assert_eq!(result[2].name, "unchanged.txt");
    }

    #[test]
    fn test_no_false_rename_for_different_blocks() {
        let old_file = FileInfo {
            name: "old.txt".to_string(),
            file_type: FileType::File,
            size: 0,
            permissions: 0,
            modified_s: 0,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 1,
            block_size: 0,
            blocks: vec![BlockInfo {
                size: 10,
                hash: vec![1, 2, 3],
                offset: 0,
            }],
            symlink_target: None,
            deleted: Some(true),
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        };

        let new_file = FileInfo {
            name: "new.txt".to_string(),
            file_type: FileType::File,
            size: 10,
            permissions: 0,
            modified_s: 0,
            modified_ns: 0,
            version: Vector::new(),
            sequence: 2,
            block_size: 0,
            blocks: vec![BlockInfo {
                size: 10,
                hash: vec![7, 8, 9], // 不同哈希
                offset: 0,
            }],
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        };

        let input = vec![old_file, new_file];
        let result = detect_and_reorder_renames(input);

        // 顺序不变
        assert_eq!(result[0].name, "old.txt");
        assert_eq!(result[1].name, "new.txt");
    }

    #[tokio::test]
    async fn test_sub_scan_preserves_relative_path() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let scanner = Scanner::new(db, events, 1);

        let temp_dir = tempfile::tempdir().unwrap();
        let subdir = temp_dir.path().join("subdir");
        tokio::fs::create_dir(&subdir).await.unwrap();
        tokio::fs::write(subdir.join("hello.md"), b"hello")
            .await
            .unwrap();

        let folder = Folder::new("test", temp_dir.path().to_str().unwrap());
        let files = scanner.scan_folder_sub(&folder, "subdir").await.unwrap();

        // W16 回归：子扫描返回的文件名必须以 folder 根为基准，不能截断成 basename。
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(
            names.contains(&"subdir/hello.md"),
            "expected subdir/hello.md, got {:?}",
            names
        );
        assert!(
            !names.contains(&"hello.md"),
            "sub-scan must not produce root-level hello.md"
        );
    }

    #[tokio::test]
    async fn test_mark_deleted_subtree_only_deletes_missing_files() {
        use syncthing_core::types::Vector;

        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let scanner = Scanner::new(db.clone(), events, 1);

        let temp_dir = tempfile::tempdir().unwrap();
        let subdir = temp_dir.path().join("subdir");
        tokio::fs::create_dir(&subdir).await.unwrap();
        tokio::fs::write(subdir.join("keep.md"), b"keep")
            .await
            .unwrap();
        // gone.md 只进 DB，磁盘上不存在
        let gone = FileInfo {
            name: "subdir/gone.md".to_string(),
            file_type: syncthing_core::types::FileType::File,
            size: 4,
            permissions: 0o644,
            modified_s: 1,
            modified_ns: 0,
            version: Vector::new().with_counter(1, 1),
            sequence: 1,
            block_size: 0,
            blocks: vec![],
            symlink_target: None,
            deleted: None,
            modified_by: None,
            blocks_hash: None,
            no_permissions: None,
            base_version: None,
        };
        db.update_files("test", vec![gone]).await.unwrap();

        let folder = Folder::new("test", temp_dir.path().to_str().unwrap());
        let deleted = scanner
            .mark_deleted_subtree(&folder, "subdir")
            .await
            .unwrap();

        // W16 回归：子树删除检查必须跳过仍存在的文件，否则会把子树内所有文件标删除。
        let deleted_names: Vec<&str> = deleted.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(deleted_names, vec!["subdir/gone.md"]);
    }
}
