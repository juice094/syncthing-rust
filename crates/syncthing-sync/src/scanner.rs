//! 文件夹扫描器
//!
//! 实现定期扫描本地文件夹变更的功能

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

/// 块大小（128KB，与 Syncthing 默认一致）
const DEFAULT_BLOCK_SIZE: i32 = syncthing_core::constants::DEFAULT_BLOCK_SIZE;

/// Scanner 默认排除的文件/目录名（与 Go Syncthing 对齐）
/// 这些条目在 .stignore 加载前生效，防止元数据被索引和同步。
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

/// Scanner 默认排除的文件后缀模式
const DEFAULT_IGNORED_SUFFIXES: &[&str] = &[".syncthing.tmp", "~syncthing~"];

/// 相对路径的任一组成部分是否为默认排除的元数据名（.stversions/.sttrash 等）。
/// watcher 增量路径（scan_changed_file / mark_deleted_subtree / sub-scan 根）
/// 都必须过这道拦截，否则元数据会进入索引并同步，形成归档↔同步反馈循环。
fn is_default_ignored_path(relative_path: &str) -> bool {
    relative_path
        .split(['/', '\\'])
        .any(|c| DEFAULT_IGNORED_NAMES.iter().any(|&ignored| c == ignored))
}

/// 文件夹扫描器
pub struct Scanner {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    /// 本地设备的版本向量计数器 ID（来自真实设备 ID，禁止硬编码为 1，
    /// 否则双端离线并发修改会被误判为线性历史，删除方静默覆盖对端修改）
    local_counter_id: u64,
}

impl Scanner {
    /// 创建新的扫描器
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher, local_counter_id: u64) -> Self {
        Self {
            db,
            events,
            local_counter_id,
        }
    }

    /// 扫描单个文件夹
    pub async fn scan_folder(&self, folder: &Folder) -> Result<Vec<FileInfo>> {
        self.scan_folder_at(folder, None).await
    }

    /// 扫描文件夹中的子目录
    pub async fn scan_folder_sub(&self, folder: &Folder, sub: &str) -> Result<Vec<FileInfo>> {
        self.scan_folder_at(folder, Some(sub)).await
    }

    async fn scan_folder_at(&self, folder: &Folder, sub: Option<&str>) -> Result<Vec<FileInfo>> {
        // watcher 增量路径会把变动所在子目录作为扫描根传入；若扫描根本身落在
        // 默认排除的元数据目录内（典型：versioner 写 .stversions 触发 watcher），
        // 逐条目按文件名过滤无法拦截（归档文件名不含被排除名），必须在入口拒绝，
        // 否则归档文件会被索引并同步，形成归档↔同步反馈循环。
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

        // 加载 .stignore（如果存在）—— 始终以 folder 根目录为基准
        let ignore_path = path.join(".stignore");
        let mut matcher = IgnoreMatcher::load(&ignore_path);
        let file_rule_count = matcher.len();

        // 同时加载配置中的 ignore_patterns（D-2 修复：此前完全未使用）
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

        // 递归扫描目录
        match self
            .scan_directory(
                &folder.id,
                path,
                &scan_root,
                Path::new(""),
                &mut visited_paths,
                &matcher,
            )
            .await
        {
            Ok(files) => {
                // 仅全量扫描时检查已删除的文件
                if sub.is_none() {
                    let db_files = self.db.get_folder_files(&folder.id).await?;
                    for db_file in db_files {
                        let full_path = path.join(&db_file.name);
                        if !full_path.exists() && !db_file.is_deleted() {
                            // 检查是否正在下载中（临时文件存在）
                            let temp_path = temp_path_for(&full_path);
                            if temp_path.exists() {
                                debug!(file = %db_file.name, "File is being downloaded, skipping deleted check");
                                continue;
                            }
                            debug!(file = %db_file.name, "File was deleted");
                            let mut deleted_info = db_file.clone();
                            deleted_info.deleted = Some(true);
                            // NOTE: 暂时保留 blocks 用于重命名检测，检测后再清空
                            deleted_info.size = 0;
                            deleted_info.sequence = self.db.increment_sequence(&folder.id).await?;
                            deleted_info.version.increment(self.local_counter_id);
                            changed_files.push(deleted_info);
                        }
                    }
                }

                // 检查变更的文件
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
                                // 保留上一次全局一致的 base 版本
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

                // P1: 重命名检测——新文件与最近删除的文件块哈希相同
                if sub.is_none() {
                    changed_files = detect_and_reorder_renames(changed_files);
                }

                // 清空已删除文件的 blocks（BEP 协议要求）
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

        // 更新数据库
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

    /// 递归扫描目录
    ///
    /// `relative_prefix` 随递归逐层累加，彻底避免 `Path::strip_prefix` 在 Windows UNC/混合分隔符下的平台差异。
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

            // 跳过隐藏文件和特殊文件
            let file_name_os = match path.file_name() {
                Some(name) => name,
                None => continue,
            };
            let name = file_name_os.to_string_lossy();

            // 默认排除 syncthing 元数据（D-1 修复）
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
            // 跳过 Syncthing 冲突文件
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

            // 随递归逐层构建相对路径，避免平台相关的 strip_prefix
            let relative_path = if relative_prefix.as_os_str().is_empty() {
                name.replace('\\', "/")
            } else {
                format!(
                    "{}/{}",
                    relative_prefix.to_string_lossy().replace('\\', "/"),
                    name.replace('\\', "/")
                )
            };

            // 应用 .stignore 规则
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
                // 递归扫描子目录
                let mut next_prefix = relative_prefix.to_path_buf();
                next_prefix.push(file_name_os);
                let sub_files = self
                    .scan_directory(folder_id, _base_path, &path, &next_prefix, visited, matcher)
                    .await?;
                files.extend(sub_files);

                // 添加目录条目
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
                // 计算文件哈希和块信息
                let file_info = self
                    .scan_file(&path, &relative_path, &metadata, folder_id)
                    .await?;
                files.push(file_info);
            } else if metadata.is_symlink() {
                // 处理符号链接
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

    /// 扫描单个文件
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

        // 计算块信息，同时计算整体内容哈希
        let mut blocks = Vec::new();
        let mut content_hasher = Sha256::new();
        if size > 0 {
            let block_size = DEFAULT_BLOCK_SIZE;
            let num_blocks = ((size + block_size as i64 - 1) / block_size as i64) as usize;

            // 对于大文件，异步读取并计算哈希
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
            // BEP 兼容：零字节文件也必须有一个 block，hash 为 SHA256("")
            // 否则 Syncthing-Fork 等客户端会报 "file with empty block list" 并 Close。
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

    /// 检查文件是否变更
    fn has_file_changed(old: &FileInfo, new: &FileInfo) -> bool {
        // 内容哈希优先：块列表一致即视为未变更，不再比较 mtime/权限。
        // 跨平台同步时 mtime 精度（ns vs FILETIME 100ns）和权限位（0o660 vs
        // OS 默认）必然有噪声，先比元数据会造成"扫描→版本递增→对端回拉→
        // 再扫描"的无限版本乒乓（两边内容相同却互相覆盖，还会吞掉真实编辑）。
        // ponytail: mtime/权限的纯元数据变更暂不传播；升级路径是按 Go
        // Syncthing 的平台规则显式处理 metadata-only 更新。
        if old.blocks.len() == new.blocks.len()
            && old
                .blocks
                .iter()
                .zip(new.blocks.iter())
                .all(|(o, n)| o.hash == n.hash)
        {
            return false;
        }

        // 检查大小
        if old.size != new.size {
            return true;
        }

        // 检查修改时间
        if old.modified_s != new.modified_s || old.modified_ns != new.modified_ns {
            return true;
        }

        // 检查权限
        if old.permissions != new.permissions {
            return true;
        }

        true
    }

    /// 检查两个文件的块哈希列表是否完全相同（用于重命名检测）
    /// 扫描单个相对路径（用于增量扫描）。
    /// 返回变更的 FileInfo；如果文件未变更或不存在，返回 None。
    pub async fn scan_changed_file(
        &self,
        folder: &Folder,
        relative_path: &str,
    ) -> Result<Option<FileInfo>> {
        // 元数据目录（.stversions/.sttrash 等）内的变更不进入索引
        if is_default_ignored_path(relative_path) {
            trace!(path = %relative_path, "Skipping default-ignored metadata path");
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

    /// 将单个相对路径标记为已删除（如果它存在于 DB 中且尚未删除）。
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

    /// 将某个子树下的所有 DB 文件标记为已删除（用于增量扫描发现子树内删除）。
    pub async fn mark_deleted_subtree(
        &self,
        folder: &Folder,
        relative_prefix: &str,
    ) -> Result<Vec<FileInfo>> {
        // 元数据目录下的"删除"不广播（从未被索引，且传播会让对端跟着删 .sttrash）
        if is_default_ignored_path(relative_prefix) {
            return Ok(Vec::new());
        }

        let prefix_with_slash = if relative_prefix.ends_with('/') {
            relative_prefix.to_string()
        } else {
            format!("{}/", relative_prefix)
        };
        let db_files = self.db.get_folder_files(&folder.id).await?;
        let mut deleted = Vec::new();
        for db_file in db_files {
            if db_file.is_deleted() {
                continue;
            }
            if db_file.name == relative_prefix || db_file.name.starts_with(&prefix_with_slash) {
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

    /// 快速扫描（仅检查修改时间）
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
                    // 文件已被删除（但检查是否正在下载中）
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
}
