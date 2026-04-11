//! 索引处理器
//! 
//! 处理接收到的远程索引和索引更新消息

use crate::conflict_resolver::{ConflictResolver, VersionComparison};
use crate::database::LocalDatabase;
use crate::error::{Result, SyncError};
use crate::events::{EventPublisher, SyncEvent};
use syncthing_core::types::{FileInfo, Index, IndexUpdate, Folder};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, trace};

/// 索引处理器
pub struct IndexHandler {
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    conflict_resolver: ConflictResolver,
}

impl IndexHandler {
    /// 创建新的索引处理器
    pub fn new(db: Arc<dyn LocalDatabase>, events: EventPublisher) -> Self {
        let conflict_resolver = ConflictResolver::new(db.clone(), events.clone());
        Self {
            db,
            events,
            conflict_resolver,
        }
    }

    /// 处理完整索引
    pub async fn handle_index(
        &self,
        folder: &Folder,
        device: syncthing_core::DeviceId,
        index: Index,
    ) -> Result<Vec<FileInfo>> {
        info!(
            folder = %folder.id,
            device = %device.short_id(),
            file_count = index.files.len(),
            "Received full index"
        );

        if index.folder != folder.id {
            return Err(SyncError::index(format!(
                "Folder mismatch: expected {}, got {}",
                folder.id, index.folder
            )));
        }

        // 处理索引文件
        self.process_files(folder, device, &index.files, true).await
    }

    /// 处理索引更新
    pub async fn handle_index_update(
        &self,
        folder: &Folder,
        device: syncthing_core::DeviceId,
        update: IndexUpdate,
    ) -> Result<Vec<FileInfo>> {
        info!(
            folder = %folder.id,
            device = %device.short_id(),
            file_count = update.files.len(),
            "Received index update"
        );

        if update.folder != folder.id {
            return Err(SyncError::index(format!(
                "Folder mismatch: expected {}, got {}",
                folder.id, update.folder
            )));
        }

        // 处理更新的文件
        let needed = self.process_files(folder, device, &update.files, false).await?;

        // 发布事件
        self.events.publish(SyncEvent::RemoteIndexReceived {
            folder: folder.id.clone(),
            device,
            files: update.files,
        });

        Ok(needed)
    }

    /// 处理文件列表
    async fn process_files(
        &self,
        folder: &Folder,
        _device: syncthing_core::DeviceId,
        files: &[FileInfo],
        is_full_index: bool,
    ) -> Result<Vec<FileInfo>> {
        let mut needed_files = Vec::new();

        for remote_file in files {
            trace!(file = %remote_file.name, "Processing remote file info");

            match self.db.get_file(&folder.id, &remote_file.name).await? {
                Some(local_file) => {
                    // 检查是否需要更新
                    match self.needs_update(&local_file, remote_file).await? {
                        UpdateDecision::Update => {
                            debug!(file = %remote_file.name, "File needs update");
                            
                            // 检查冲突
                            if self.conflict_resolver.is_conflict(&local_file, remote_file) {
                                let folder_path = Path::new(&folder.path);
                                self.conflict_resolver.resolve_conflict(
                                    &folder.id,
                                    &local_file,
                                    remote_file,
                                    folder_path,
                                ).await?;
                            } else {
                                // 无冲突，直接更新
                                self.db.update_file(&folder.id, remote_file.clone()).await?;
                                
                                // 检查是否需要下载
                                if !remote_file.is_deleted() {
                                    needed_files.push(remote_file.clone());
                                }
                            }
                        }
                        UpdateDecision::Ignore => {
                            trace!(file = %remote_file.name, "Local version is newer, ignoring");
                        }
                        UpdateDecision::Conflict => {
                            debug!(file = %remote_file.name, "Conflict detected");
                            let folder_path = Path::new(&folder.path);
                            self.conflict_resolver.resolve_conflict(
                                &folder.id,
                                &local_file,
                                remote_file,
                                folder_path,
                            ).await?;
                        }
                    }
                }
                None => {
                    // 本地没有此文件
                    if !remote_file.is_deleted() {
                        debug!(file = %remote_file.name, "New file from remote");
                        self.db.update_file(&folder.id, remote_file.clone()).await?;
                        needed_files.push(remote_file.clone());
                    } else {
                        // 远程删除，本地没有，添加删除标记
                        debug!(file = %remote_file.name, "Recording remote deletion");
                        self.db.update_file(&folder.id, remote_file.clone()).await?;
                    }
                }
            }
        }

        // 如果是完整索引，检查本地是否有远程不存在的文件
        if is_full_index {
            let local_files = self.db.get_folder_files(&folder.id).await?;
            for local_file in local_files {
                if !files.iter().any(|f| f.name == local_file.name) {
                    // 远程没有这个文件
                    if !local_file.is_deleted() {
                        debug!(file = %local_file.name, "File not in remote index, may need to upload");
                    }
                }
            }
        }

        Ok(needed_files)
    }

    /// 判断是否需要更新
    async fn needs_update(&self, local: &FileInfo, remote: &FileInfo) -> Result<UpdateDecision> {
        // 比较版本向量
        match self.conflict_resolver.compare_versions(&local.version, &remote.version) {
            VersionComparison::Equal => {
                // 版本相同，检查其他属性
                if local.size != remote.size
                    || local.modified_s != remote.modified_s
                    || local.modified_ns != remote.modified_ns
                {
                    // 版本相同但属性不同，可能是冲突
                    Ok(UpdateDecision::Conflict)
                } else {
                    Ok(UpdateDecision::Ignore)
                }
            }
            VersionComparison::Greater => {
                // 本地版本更新
                Ok(UpdateDecision::Ignore)
            }
            VersionComparison::Less => {
                // 远程版本更新
                Ok(UpdateDecision::Update)
            }
            VersionComparison::Conflict => {
                // 版本向量不可比较
                Ok(UpdateDecision::Conflict)
            }
        }
    }

    /// 计算索引差异
    pub async fn calculate_diff(&self, folder: &str, remote_files: &[FileInfo]) -> Result<IndexDiff> {
        let local_files: Vec<syncthing_core::types::FileInfo> = self.db.get_folder_files(folder).await?;
        let mut diff = IndexDiff::default();

        // 检查远程有哪些本地没有的或更新的文件
        for remote in remote_files {
            match local_files.iter().find(|l| l.name == remote.name) {
                Some(local) => {
                    match self.needs_update(local, remote).await? {
                        UpdateDecision::Update => {
                            diff.to_download.push(remote.clone());
                        }
                        UpdateDecision::Conflict => {
                            diff.conflicts.push((local.clone(), remote.clone()));
                        }
                        UpdateDecision::Ignore => {
                            // 本地更新，可能需要上传
                            diff.to_upload.push(local.clone());
                        }
                    }
                }
                None => {
                    if !remote.is_deleted() {
                        diff.to_download.push(remote.clone());
                    }
                }
            }
        }

        // 检查本地有哪些远程没有的
        for local in local_files {
            if !remote_files.iter().any(|r| r.name == local.name) && !local.is_deleted() {
                diff.to_upload.push(local);
            }
        }

        Ok(diff)
    }

    /// 生成本地索引更新
    pub async fn generate_index_update(&self, folder: &str, since_sequence: u64) -> Result<Vec<FileInfo>> {
        let needed_files = self.db.get_needed_files(folder, since_sequence).await?;
        Ok(needed_files)
    }

    /// 检查全局状态
    pub async fn check_globals(&self, folder: &str, name: &str) -> Result<Vec<FileInfo>> {
        self.db.check_globals(folder, name).await
    }

    /// 合并索引（用于处理多个设备的索引）
    pub fn merge_indexes(&self, indexes: Vec<Index>) -> Index {
        if indexes.is_empty() {
            return Index {
                folder: String::new(),
                files: vec![],
            };
        }

        let folder = indexes[0].folder.clone();
        let mut file_map = std::collections::HashMap::new();

        for index in indexes {
            for file in index.files {
                // 对于每个文件，选择版本最新的
                match file_map.get(&file.name) {
                    Some(existing) => {
                        if self.conflict_resolver.select_winner(existing, &file) == &file {
                            file_map.insert(file.name.clone(), file);
                        }
                    }
                    None => {
                        file_map.insert(file.name.clone(), file);
                    }
                }
            }
        }

        Index {
            folder,
            files: file_map.into_values().collect(),
        }
    }
}

/// 更新决策
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateDecision {
    Update,
    Ignore,
    Conflict,
}

/// 索引差异
#[derive(Debug, Clone, Default)]
pub struct IndexDiff {
    pub to_download: Vec<FileInfo>,
    pub to_upload: Vec<FileInfo>,
    pub conflicts: Vec<(FileInfo, FileInfo)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryDatabase;
    use syncthing_core::types::{Vector, FileType};

    fn create_test_file(name: &str, version: Vector) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            file_type: FileType::File,
            size: 100,
            permissions: 0o644,
            modified_s: 1234567890,
            modified_ns: 0,
            version,
            sequence: 1,
            block_size: 128 * 1024,
            blocks: vec![],
            symlink_target: None,
            deleted: None,
        }
    }

    #[tokio::test]
    async fn test_index_processing() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let handler = IndexHandler::new(db.clone(), events);

        let folder = Folder::new("test", "/tmp/test");
        let device = syncthing_core::DeviceId::random();

        let remote_file = create_test_file("test.txt", Vector::new().with_counter(1, 5));
        
        let index = Index {
            folder: "test".to_string(),
            files: vec![remote_file.clone()],
        };

        let needed = handler.handle_index(&folder, device, index).await.unwrap();
        assert_eq!(needed.len(), 1);
        assert_eq!(needed[0].name, "test.txt");
    }

    #[tokio::test]
    async fn test_update_when_remote_newer() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let handler = IndexHandler::new(db.clone(), events);

        // 先添加本地文件
        let local_file = create_test_file("test.txt", Vector::new().with_counter(1, 3));
        db.update_file("test", local_file).await.unwrap();

        let folder = Folder::new("test", "/tmp/test");
        let device = syncthing_core::DeviceId::random();

        // 远程版本更新
        let remote_file = create_test_file("test.txt", Vector::new().with_counter(1, 5));
        let update = IndexUpdate {
            folder: "test".to_string(),
            files: vec![remote_file],
        };

        let needed = handler.handle_index_update(&folder, device, update).await.unwrap();
        assert_eq!(needed.len(), 1);
    }

    #[tokio::test]
    async fn test_ignore_when_local_newer() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let handler = IndexHandler::new(db.clone(), events);

        // 先添加本地文件（版本更新）
        let local_file = create_test_file("test.txt", Vector::new().with_counter(1, 5));
        db.update_file("test", local_file).await.unwrap();

        let folder = Folder::new("test", "/tmp/test");
        let device = syncthing_core::DeviceId::random();

        // 远程版本更旧
        let remote_file = create_test_file("test.txt", Vector::new().with_counter(1, 3));
        let update = IndexUpdate {
            folder: "test".to_string(),
            files: vec![remote_file],
        };

        let needed = handler.handle_index_update(&folder, device, update).await.unwrap();
        assert!(needed.is_empty());
    }
}
