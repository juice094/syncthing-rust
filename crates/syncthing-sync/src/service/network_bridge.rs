//! Network-layer bridge for `SyncService`.
//!
//! Extracted from `mod.rs` in T2.3 (see `docs/drafts/RFC-001-service-split.md`).
//! Holds the callbacks invoked by the BEP transport (`syncthing-net`):
//! `handle_index`, `handle_index_update`, `handle_block_request`,
//! `generate_index_update`, plus a few read-only accessors used by the same
//! call sites. No business changes.

use crate::block_server;
use crate::error::{Result, SyncError};
use crate::folder_model::FolderModel;
use crate::service::SyncService;
use std::sync::Arc;
use syncthing_core::types::{FileInfo, Index, IndexUpdate};
use syncthing_core::DeviceId;

impl SyncService {
    /// 处理接收到的索引消息（供网络层调用）
    pub async fn handle_index(
        &self,
        folder_id: &str,
        device: DeviceId,
        index: Index,
    ) -> Result<Vec<FileInfo>> {
        let folder_model = self
            .folders
            .get(folder_id)
            .ok_or_else(|| SyncError::FolderNotFound(folder_id.to_string()))?;

        let needed: Vec<syncthing_core::types::FileInfo> = self
            .index_handler
            .handle_index(folder_model.config(), device, index)
            .await?;

        // 触发文件夹的远程索引处理
        folder_model
            .handle_remote_index(device, needed.clone())
            .await?;

        // Update peer sync state for completion tracking
        let key = (device, folder_id.to_string());
        self.peer_sync_states.insert(key, needed.len());

        Ok(needed)
    }

    /// 处理接收到的索引更新（供网络层调用）
    pub async fn handle_index_update(
        &self,
        folder_id: &str,
        device: DeviceId,
        update: IndexUpdate,
    ) -> Result<Vec<FileInfo>> {
        let folder_model = self
            .folders
            .get(folder_id)
            .ok_or_else(|| SyncError::FolderNotFound(folder_id.to_string()))?;

        let needed: Vec<syncthing_core::types::FileInfo> = self
            .index_handler
            .handle_index_update(folder_model.config(), device, update)
            .await?;

        // 触发文件夹的远程索引处理
        folder_model
            .handle_remote_index(device, needed.clone())
            .await?;

        // Update peer sync state for completion tracking
        let key = (device, folder_id.to_string());
        self.peer_sync_states.insert(key, needed.len());

        Ok(needed)
    }

    /// 处理远程块请求（供网络层调用）
    pub async fn handle_block_request(
        &self,
        req: &bep_protocol::messages::Request,
    ) -> std::result::Result<Vec<u8>, block_server::BlockRequestError> {
        let config = self.config.read().await;
        let folder = config.folders.iter().find(|f| f.id == req.folder);
        let folder_path = match folder {
            Some(f) => std::path::PathBuf::from(&f.path),
            None => return Err(block_server::BlockRequestError::FolderNotFound),
        };
        drop(config);
        block_server::serve_block_request(&folder_path, req).await
    }

    /// 生成索引更新（供网络层调用）
    pub async fn generate_index_update(
        &self,
        folder_id: &str,
        since_sequence: u64,
    ) -> Result<Vec<FileInfo>> {
        self.index_handler
            .generate_index_update(folder_id, since_sequence)
            .await
    }

    /// 获取所有文件夹ID
    pub fn get_folder_ids(&self) -> Vec<String> {
        self.folders.iter().map(|e| e.key().clone()).collect()
    }

    /// 获取文件夹模型
    pub fn get_folder(&self, folder_id: &str) -> Option<Arc<FolderModel>> {
        self.folders.get(folder_id).map(|f| f.clone())
    }

    /// 获取某个文件夹相对于某个设备的同步完成度（needed files 数量）
    pub fn get_folder_completion(&self, device_id: DeviceId, folder_id: &str) -> usize {
        self.peer_sync_states
            .get(&(device_id, folder_id.to_string()))
            .map(|v| *v)
            .unwrap_or(0)
    }
}
