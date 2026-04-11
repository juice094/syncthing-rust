//! Module: syncthing-sync
//! Worker: Agent-Integration
//! Status: UNVERIFIED
//!
//! ⚠️ 此代码由Agent生成，未经主控验证
//!
//! Scan Scheduler - 扫描调度器
//!
//! 负责定期扫描文件夹，检测本地文件变化并更新索引。


use std::sync::Arc;
use std::time::Duration;

use tokio::time::interval;
use tracing::{debug, error, info, warn};

use syncthing_core::traits::{FileSystem, SyncModel};
use syncthing_core::types::{FileInfo, FolderConfig, FolderId};
use syncthing_core::Result;

/// 扫描调度器
///
/// 管理所有文件夹的定期扫描任务
#[derive(Clone)]
pub struct ScanScheduler {
    /// 文件夹配置列表
    folders: Vec<FolderConfig>,
    /// 文件系统抽象
    filesystem: Arc<dyn FileSystem>,
    /// 同步引擎（用于执行扫描）
    sync_engine: Arc<dyn SyncModel>,
}

impl ScanScheduler {
    /// 创建新的扫描调度器
    ///
    /// # Arguments
    /// * `folders` - 文件夹配置列表
    /// * `filesystem` - 文件系统实现
    /// * `sync_engine` - 同步引擎
    pub fn new(
        folders: Vec<FolderConfig>,
        filesystem: Arc<dyn FileSystem>,
        sync_engine: Arc<dyn SyncModel>,
    ) -> Self {
        Self {
            folders,
            filesystem,
            sync_engine,
        }
    }

    /// 运行调度器
    ///
    /// 为每个文件夹启动定期扫描任务
    pub async fn run(&self) {
        info!("启动扫描调度器，{} 个文件夹", self.folders.len());

        for folder in &self.folders {
            let folder = folder.clone();
            let filesystem = self.filesystem.clone();
            let sync_engine = self.sync_engine.clone();

            tokio::spawn(async move {
                // 初始扫描（启动时执行一次）
                info!("执行初始扫描: {}", folder.id);
                if let Err(e) = scan_folder(&folder, &filesystem, &sync_engine).await {
                    warn!("初始扫描失败: {} - {}", folder.id, e);
                }

                // 设置定期扫描间隔
                let interval_secs = folder.rescan_interval_secs.max(10); // 最小 10 秒
                let mut ticker = interval(Duration::from_secs(interval_secs as u64));

                loop {
                    ticker.tick().await;

                    debug!("执行定期扫描: {}", folder.id);
                    if let Err(e) = scan_folder(&folder, &filesystem, &sync_engine).await {
                        warn!("定期扫描失败: {} - {}", folder.id, e);
                    }
                }
            });
        }

        // 保持运行（任务在后台执行）
        // 使用一个永远不会完成的 future
        futures::future::pending::<()>().await;
    }

    /// 立即扫描指定文件夹
    pub async fn scan_now(&self, folder_id: &FolderId) -> Result<()> {
        let folder = self
            .folders
            .iter()
            .find(|f| &f.id == folder_id)
            .ok_or_else(|| syncthing_core::SyncthingError::FolderNotFound(folder_id.clone()))?;

        scan_folder(folder, &self.filesystem, &self.sync_engine).await
    }

    /// 获取文件夹数量
    pub fn folder_count(&self) -> usize {
        self.folders.len()
    }
}

/// 扫描单个文件夹
///
/// 1. 设置状态为 Scanning
/// 2. 扫描文件系统
/// 3. 对比上次索引
/// 4. 生成 IndexUpdate
/// 5. 发送给所有连接的设备
/// 6. 更新本地索引
async fn scan_folder(
    folder: &FolderConfig,
    _filesystem: &Arc<dyn FileSystem>,
    sync_engine: &Arc<dyn SyncModel>,
) -> Result<()> {
    let folder_id = &folder.id;
    let folder_path = &folder.path;

    info!("开始扫描文件夹: {} -> {:?}", folder_id, folder_path);

    // 检查路径是否存在
    if !folder_path.exists() {
        warn!("文件夹路径不存在: {:?}", folder_path);
        // 创建文件夹
        tokio::fs::create_dir_all(folder_path)
            .await
            .map_err(|e| syncthing_core::SyncthingError::Io(e.to_string()))?;
        info!("创建文件夹: {:?}", folder_path);
    }

    // 执行扫描
    let start_time = std::time::Instant::now();

    // 调用同步引擎的扫描功能
    // SyncEngine 实现了 SyncModel trait
    // 扫描过程会更新索引并广播变化
    match sync_engine.scan_folder(folder_id).await {
        Ok(()) => {
            info!(
                "扫描完成: {}，耗时 {:?}",
                folder_id,
                start_time.elapsed()
            );
            Ok(())
        }
        Err(e) => {
            error!("扫描失败: {} - {}", folder_id, e);
            Err(e)
        }
    }
}

/// 比较文件列表，找出变化
#[allow(dead_code)]
fn diff_files(
    old_files: &[FileInfo],
    new_files: &[FileInfo],
) -> (Vec<FileInfo>, Vec<String>) {
    let mut changed = Vec::new();
    let mut deleted = Vec::new();

    // 找出新文件和修改的文件
    for new_file in new_files {
        match old_files.iter().find(|f| f.name == new_file.name) {
            Some(old_file) => {
                // 检查是否修改
                if old_file.modified != new_file.modified
                    || old_file.size != new_file.size
                    || old_file.blocks.len() != new_file.blocks.len()
                {
                    changed.push(new_file.clone());
                }
            }
            None => {
                // 新文件
                changed.push(new_file.clone());
            }
        }
    }

    // 找出删除的文件
    for old_file in old_files {
        if !new_files.iter().any(|f| f.name == old_file.name) {
            deleted.push(old_file.name.clone());
        }
    }

    (changed, deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[allow(dead_code)]
    fn create_test_folder_config() -> FolderConfig {
        FolderConfig {
            id: FolderId::new("test"),
            label: "Test Folder".to_string(),
            path: PathBuf::from("/tmp/test"),
            devices: vec![],
            rescan_interval_secs: 3600,
            versioning: syncthing_core::types::VersioningConfig::None,
        }
    }

    #[test]
    fn test_diff_files() {
        let old_files = vec![
            FileInfo::new("file1.txt"),
            FileInfo::new("file2.txt"),
        ];

        let mut new_files = vec![
            FileInfo::new("file1.txt"),
            FileInfo::new("file3.txt"),
        ];
        // 修改 file1.txt
        new_files[0].size = 100;

        let (changed, deleted) = diff_files(&old_files, &new_files);

        assert_eq!(changed.len(), 2); // file1 (modified) + file3 (new)
        assert_eq!(deleted.len(), 1); // file2 (deleted)
        assert!(deleted.contains(&"file2.txt".to_string()));
    }
}
