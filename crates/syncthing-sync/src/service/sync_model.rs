//! `syncthing_core::traits::SyncModel` trait implementation for `SyncService`.
//!
//! Extracted from `mod.rs` in T2.3 (see `docs/drafts/RFC-001-service-split.md`).
//! This implementation is the FFI / cross-crate boundary exposing `SyncService`
//! via the abstract `SyncModel` trait defined in `syncthing-core`. No business
//! changes.

use crate::service::SyncService;
use tracing::info;

#[async_trait::async_trait]
impl syncthing_core::traits::SyncModel for SyncService {
    async fn start_folder(&self, folder: syncthing_core::FolderId) -> syncthing_core::Result<()> {
        let folder_id = folder.as_str();
        self.start_folder_internal(folder_id)
            .await
            .map_err(|e| syncthing_core::SyncthingError::internal(e.to_string()))
    }

    async fn stop_folder(&self, folder: syncthing_core::FolderId) -> syncthing_core::Result<()> {
        let folder_id = folder.as_str();

        // 检查是否在运行
        let handles = self.folder_tasks.get(folder_id).ok_or_else(|| {
            syncthing_core::SyncthingError::internal(format!("Folder not running: {}", folder_id))
        })?;

        // 发送停止信号
        handles.shutdown_tx.send(true).ok();
        drop(handles);

        // 等待任务完成并移除
        if let Some((_, handles)) = self.folder_tasks.remove(folder_id) {
            let _ = handles.scan_handle.await;
            let _ = handles.pull_handle.await;
            let _ = handles.watcher_handle.await;
        }

        info!(folder_id = %folder_id, "Folder stopped");
        Ok(())
    }

    async fn scan_folder(&self, folder: &syncthing_core::FolderId) -> syncthing_core::Result<()> {
        crate::model::SyncManager::scan_folder(self, folder.as_str())
            .await
            .map_err(|e| syncthing_core::SyncthingError::internal(e.to_string()))
    }

    async fn scan_folder_sub(
        &self,
        folder: &syncthing_core::FolderId,
        sub: &str,
    ) -> syncthing_core::Result<()> {
        crate::model::SyncManager::scan_folder_sub(self, folder.as_str(), sub)
            .await
            .map_err(|e| syncthing_core::SyncthingError::internal(e.to_string()))
    }

    async fn pull(
        &self,
        folder: &syncthing_core::FolderId,
    ) -> syncthing_core::Result<syncthing_core::traits::SyncResult> {
        crate::model::SyncManager::pull_folder(self, folder.as_str())
            .await
            .map_err(|e| syncthing_core::SyncthingError::internal(e.to_string()))?;
        Ok(syncthing_core::traits::SyncResult {
            files_processed: 0,
            bytes_transferred: 0,
            errors: vec![],
        })
    }

    async fn folder_status(
        &self,
        folder: &syncthing_core::FolderId,
    ) -> syncthing_core::Result<syncthing_core::traits::FolderStatus> {
        match self.get_folder(folder.as_str()) {
            Some(folder_model) => {
                let state = folder_model.state().await;
                let status = match state.status {
                    syncthing_core::types::FolderStatus::Idle
                    | syncthing_core::types::FolderStatus::ScanWaiting
                    | syncthing_core::types::FolderStatus::SyncWaiting
                    | syncthing_core::types::FolderStatus::Synced => {
                        syncthing_core::traits::FolderStatus::Idle
                    }
                    syncthing_core::types::FolderStatus::Scanning => {
                        syncthing_core::traits::FolderStatus::Scanning
                    }
                    syncthing_core::types::FolderStatus::Pulling
                    | syncthing_core::types::FolderStatus::Pushing => {
                        syncthing_core::traits::FolderStatus::Syncing { progress: 0.0 }
                    }
                    syncthing_core::types::FolderStatus::Paused => {
                        syncthing_core::traits::FolderStatus::Paused
                    }
                    syncthing_core::types::FolderStatus::Error => {
                        syncthing_core::traits::FolderStatus::Error {
                            message: "folder error".to_string(),
                        }
                    }
                };
                Ok(status)
            }
            None => Err(syncthing_core::SyncthingError::internal(format!(
                "folder not found: {}",
                folder
            ))),
        }
    }

    async fn folder_completion(
        &self,
        folder: &syncthing_core::FolderId,
        device: syncthing_core::DeviceId,
    ) -> syncthing_core::Result<u64> {
        let needed = self.get_folder_completion(device, folder.as_str());
        // Simple completion: 100% if needed == 0, else heuristic based on total files
        let total_files = self
            .db
            .get_folder_files(folder.as_str())
            .await
            .map(|v| v.len())
            .unwrap_or(0)
            .max(needed);
        let completion = if total_files == 0 {
            100
        } else {
            (((total_files - needed) as f64 / total_files as f64) * 100.0) as u64
        };
        Ok(completion)
    }

    async fn override_folder(
        &self,
        folder: &syncthing_core::FolderId,
    ) -> syncthing_core::Result<()> {
        let folder_id = folder.as_str();
        if let Some(folder_model) = self.folders.get(folder_id) {
            folder_model.override_local_changes().await.map_err(|e| {
                syncthing_core::SyncthingError::internal(format!("override failed: {}", e))
            })
        } else {
            Err(syncthing_core::SyncthingError::internal(format!(
                "folder not found: {}",
                folder_id
            )))
        }
    }

    async fn revert_folder(&self, folder: &syncthing_core::FolderId) -> syncthing_core::Result<()> {
        let folder_id = folder.as_str();
        if let Some(folder_model) = self.folders.get(folder_id) {
            folder_model.revert_local_changes().await.map_err(|e| {
                syncthing_core::SyncthingError::internal(format!("revert failed: {}", e))
            })
        } else {
            Err(syncthing_core::SyncthingError::internal(format!(
                "folder not found: {}",
                folder_id
            )))
        }
    }
}
