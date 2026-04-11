//! 文件夹模型实现
//! 
//! 实现文件夹级别的扫描和拉取循环

use crate::database::LocalDatabase;
use crate::error::Result;
use crate::events::{EventPublisher, SyncEvent};
use crate::index_handler::IndexHandler;
use crate::model::FolderState;
use crate::puller::{Puller, BlockSource};
use crate::scanner::Scanner;
use syncthing_core::DeviceId;
use syncthing_core::types::{FileInfo, Folder, FolderStatus};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace};

/// 文件夹模型
pub struct FolderModel {
    folder: Folder,
    state: RwLock<FolderState>,
    db: Arc<dyn LocalDatabase>,
    events: EventPublisher,
    scanner: Scanner,
    puller: Puller,
    index_handler: IndexHandler,
}

impl FolderModel {
    /// 创建新的文件夹模型
    pub fn new(
        folder: Folder,
        db: Arc<dyn LocalDatabase>,
        events: EventPublisher,
        block_source: Option<Arc<dyn BlockSource>>,
    ) -> Self {
        let scanner = Scanner::new(db.clone(), events.clone());
        let puller = Puller::new(db.clone(), events.clone())
            .with_block_source(block_source);
        let index_handler = IndexHandler::new(db.clone(), events.clone());

        let folder_id = folder.id.clone();
        Self {
            folder,
            state: RwLock::new(FolderState::new(folder_id)),
            db,
            events,
            scanner,
            puller,
            index_handler,
        }
    }

    /// 获取文件夹ID
    pub fn id(&self) -> &str {
        &self.folder.id
    }

    /// 获取文件夹配置
    pub fn config(&self) -> &Folder {
        &self.folder
    }

    /// 获取文件夹状态
    pub async fn state(&self) -> FolderState {
        self.state.read().await.clone()
    }

    /// 启动扫描循环
    pub async fn start_scan_loop(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let interval_secs = if self.folder.rescan_interval_secs > 0 {
            self.folder.rescan_interval_secs as u64
        } else {
            3600 // 默认1小时
        };

        info!(
            folder_id = %self.folder.id,
            interval_secs = interval_secs,
            "Starting scan loop"
        );

        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.scan().await {
                        error!(folder_id = %self.folder.id, error = %e, "Scan failed");
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!(folder_id = %self.folder.id, "Scan loop shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// 启动拉取循环
    pub async fn start_pull_loop(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        info!(folder_id = %self.folder.id, "Starting pull loop");

        let mut interval = tokio::time::interval(Duration::from_secs(10)); // 每10秒检查一次

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.pull().await {
                        error!(folder_id = %self.folder.id, error = %e, "Pull failed");
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!(folder_id = %self.folder.id, "Pull loop shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// 执行扫描
    pub async fn scan(&self) -> Result<Vec<FileInfo>> {
        if self.folder.paused {
            debug!(folder_id = %self.folder.id, "Folder is paused, skipping scan");
            return Ok(vec![]);
        }

        let mut state = self.state.write().await;
        
        // 更新状态
        let old_status = state.status;
        state.status = FolderStatus::Scanning;
        drop(state);

        self.events.publish(SyncEvent::FolderStateChanged {
            folder: self.folder.id.clone(),
            from: old_status,
            to: FolderStatus::Scanning,
        });

        info!(folder_id = %self.folder.id, "Starting scan");

        // 执行扫描
        let changed_files = match self.scanner.scan_folder(&self.folder).await {
            Ok(files) => {
                let changed_count = files.len();
                info!(
                    folder_id = %self.folder.id,
                    changed_count = changed_count,
                    "Scan completed"
                );

                // 如果有变更，发布索引更新事件
                if !files.is_empty() {
                    self.events.publish(SyncEvent::LocalIndexUpdated {
                        folder: self.folder.id.clone(),
                        files: files.clone(),
                    });
                }

                files
            }
            Err(e) => {
                let err_str = e.to_string();
                error!(folder_id = %self.folder.id, error = %err_str, "Scan failed");
                
                let mut state = self.state.write().await;
                state.errors.push(err_str);
                
                // 恢复状态
                state.status = old_status;
                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Scanning,
                    to: old_status,
                });
                
                return Err(e);
            }
        };

        // 更新状态
        let mut state = self.state.write().await;
        state.status = FolderStatus::Idle;
        state.last_scan = Some(chrono::Utc::now());
        
        // 更新文件计数
        if let Ok(all_files) = self.db.get_folder_files(&self.folder.id).await {
            let files: &Vec<syncthing_core::types::FileInfo> = &all_files;
            state.local_files = files.len();
        }

        self.events.publish(SyncEvent::FolderStateChanged {
            folder: self.folder.id.clone(),
            from: FolderStatus::Scanning,
            to: FolderStatus::Idle,
        });

        Ok(changed_files)
    }

    /// 执行拉取
    pub async fn pull(&self) -> Result<()> {
        if self.folder.paused {
            debug!(folder_id = %self.folder.id, "Folder is paused, skipping pull");
            return Ok(());
        }

        if !self.folder.folder_type.can_sync() {
            debug!(folder_id = %self.folder.id, "Folder type cannot sync, skipping pull");
            return Ok(());
        }

        let mut state = self.state.write().await;
        
        // 如果已经在拉取中，跳过
        if state.status == FolderStatus::Pulling {
            trace!(folder_id = %self.folder.id, "Already pulling, skipping");
            return Ok(());
        }

        let old_status = state.status;
        state.status = FolderStatus::Pulling;
        drop(state);

        self.events.publish(SyncEvent::FolderStateChanged {
            folder: self.folder.id.clone(),
            from: old_status,
            to: FolderStatus::Pulling,
        });

        info!(folder_id = %self.folder.id, "Starting pull");

        // 获取需要拉取的文件列表
        let needed_files: Vec<syncthing_core::types::FileInfo> = match self.puller.check_needed_files(&self.folder).await {
            Ok(files) => files,
            Err(e) => {
                error!(folder_id = %self.folder.id, error = %e, "Failed to check needed files");
                
                let mut state = self.state.write().await;
                state.status = old_status;
                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Pulling,
                    to: old_status,
                });
                
                return Err(e);
            }
        };

        if needed_files.is_empty() {
            debug!(folder_id = %self.folder.id, "No files need pulling");
            
            let mut state = self.state.write().await;
            state.status = old_status;
            self.events.publish(SyncEvent::FolderStateChanged {
                folder: self.folder.id.clone(),
                from: FolderStatus::Pulling,
                to: old_status,
            });
            
            return Ok(());
        }

        info!(
            folder_id = %self.folder.id,
            file_count = needed_files.len(),
            "Pulling files"
        );

        // 更新状态
        {
            let mut state = self.state.write().await;
            state.need_files = needed_files.len();
        }

        // 执行拉取
        match self.puller.pull_folder(&self.folder, needed_files).await {
            Ok(stats) => {
                info!(
                    folder_id = %self.folder.id,
                    succeeded = stats.files_succeeded,
                    failed = stats.files_failed,
                    "Pull completed"
                );

                // 更新状态
                let mut state = self.state.write().await;
                state.status = FolderStatus::Idle;
                state.last_pull = Some(chrono::Utc::now());
                state.need_files = 0;

                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Pulling,
                    to: FolderStatus::Idle,
                });

                self.events.publish(SyncEvent::SyncComplete {
                    folder: self.folder.id.clone(),
                    stats: crate::events::SyncStats {
                        files_added: stats.files_succeeded,
                        ..Default::default()
                    },
                });
            }
            Err(e) => {
                error!(folder_id = %self.folder.id, error = %e, "Pull failed");
                
                let mut state = self.state.write().await;
                state.status = old_status;
                state.errors.push(e.to_string());

                self.events.publish(SyncEvent::FolderStateChanged {
                    folder: self.folder.id.clone(),
                    from: FolderStatus::Pulling,
                    to: old_status,
                });
                
                return Err(e);
            }
        }

        Ok(())
    }

    /// 处理远程索引
    pub async fn handle_remote_index(
        &self,
        device: DeviceId,
        files: Vec<FileInfo>,
    ) -> Result<()> {
        debug!(
            folder_id = %self.folder.id,
            device = %device.short_id(),
            file_count = files.len(),
            "Handling remote index"
        );

        // 这里应该调用 index_handler 处理索引
        // 并更新需要的文件列表
        
        Ok(())
    }

    /// 更新文件夹配置
    pub async fn update_config(&mut self, config: Folder) {
        self.folder = config;
        self.events.publish(SyncEvent::FolderConfigUpdated {
            folder: self.folder.clone(),
        });
    }

    /// 暂停文件夹
    pub async fn pause(&self) {
        let mut state = self.state.write().await;
        state.status = FolderStatus::Idle;
        info!(folder_id = %self.folder.id, "Folder paused");
    }

    /// 恢复文件夹
    pub async fn resume(&self) {
        info!(folder_id = %self.folder.id, "Folder resumed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryDatabase;

    #[tokio::test]
    async fn test_folder_model_creation() {
        let db = MemoryDatabase::new();
        let events = EventPublisher::new(10);
        let folder = Folder::new("test", "/tmp/test");
        
        let model = FolderModel::new(folder, db, events, None);
        assert_eq!(model.id(), "test");
    }
}
