use std::sync::Arc;

use syncthing_core::DeviceId;
use syncthing_net::BepSessionHandler;
use syncthing_sync::{SyncService, SyncManager};

pub struct DaemonBepHandler {
    pub sync_service: Arc<SyncService>,
}

#[async_trait::async_trait]
impl BepSessionHandler for DaemonBepHandler {
    async fn generate_cluster_config(
        &self,
        device_id: DeviceId,
    ) -> syncthing_core::Result<bep_protocol::messages::ClusterConfig> {
        let config = self.sync_service.get_config().await.unwrap_or_default();
        let local_id = config.local_device_id.unwrap_or_default();
        let folders: Vec<bep_protocol::messages::WireFolder> = config
            .folders
            .iter()
            .filter(|f| f.devices.contains(&device_id))
            .map(|f| {
                let mut devices: Vec<bep_protocol::messages::WireDevice> = f
                    .devices
                    .iter()
                    .map(|d| bep_protocol::messages::WireDevice {
                        id: d.as_bytes().to_vec(),
                        name: String::new(),
                        addresses: vec![],
                        compression: bep_protocol::messages::Compression::Metadata as i32,
                        cert_name: String::new(),
                        max_sequence: 0,
                        introducer: false,
                        index_id: 0,
                        skip_introduction_removals: false,
                        encryption_password_token: Vec::new(),
                    })
                    .collect();
                // BEP 协议要求 ClusterConfig 的 devices 列表必须包含本地设备
                if !f.devices.contains(&local_id) {
                    devices.push(bep_protocol::messages::WireDevice {
                        id: local_id.as_bytes().to_vec(),
                        name: String::new(),
                        addresses: vec![],
                        compression: bep_protocol::messages::Compression::Metadata as i32,
                        cert_name: String::new(),
                        max_sequence: 0,
                        introducer: false,
                        index_id: 0,
                        skip_introduction_removals: false,
                        encryption_password_token: Vec::new(),
                    });
                }
                bep_protocol::messages::WireFolder {
                    id: f.id.clone(),
                    label: f.label.clone().unwrap_or_default(),
                    r#type: bep_protocol::messages::FolderType::SendReceive as i32,
                    stop_reason: bep_protocol::messages::FolderStopReason::Running as i32,
                    devices,
                }
            })
            .collect();

        Ok(bep_protocol::messages::ClusterConfig {
            folders,
            secondary: false,
        })
    }

    async fn generate_index(
        &self,
        folder_id: &str,
        _device_id: DeviceId,
    ) -> syncthing_core::Result<syncthing_core::types::Index> {
        let mut files = self.sync_service.generate_index_update(folder_id, 0).await.map_err(|e| {
            syncthing_core::SyncthingError::internal(format!("generate_index_update failed: {}", e))
        })?;
        // BEP protocol requires deleted files to have empty block lists
        for file in &mut files {
            if file.is_deleted() {
                file.blocks.clear();
            }
        }
        Ok(syncthing_core::types::Index {
            folder: folder_id.to_string(),
            files,
        })
    }

    async fn on_index(
        &self,
        device_id: DeviceId,
        index: syncthing_core::types::Index,
    ) -> syncthing_core::Result<()> {
        let folder = index.folder.clone();
        self.sync_service.handle_index(&folder, device_id, index).await.map_err(|e| {
            syncthing_core::SyncthingError::internal(format!("handle_index failed: {:?}", e))
        })?;
        Ok(())
    }

    async fn on_index_update(
        &self,
        device_id: DeviceId,
        update: syncthing_core::types::IndexUpdate,
    ) -> syncthing_core::Result<()> {
        let folder = update.folder.clone();
        self.sync_service
            .handle_index_update(&folder, device_id, update)
            .await
            .map_err(|e| {
                syncthing_core::SyncthingError::internal(format!("handle_index_update failed: {:?}", e))
            })?;
        Ok(())
    }

    async fn on_block_request(
        &self,
        _device_id: DeviceId,
        req: bep_protocol::messages::Request,
    ) -> std::result::Result<Vec<u8>, bep_protocol::messages::ErrorCode> {
        self.sync_service.handle_block_request(&req).await.map_err(|e| e.error_code())
    }
}
