//! `SyncManager` trait implementation for `SyncService`.
//!
//! Extracted from `mod.rs` in T2.3 (see `docs/drafts/RFC-001-service-split.md`).
//! Holds public CRUD operations exposed via the `SyncManager` trait — devices,
//! folders, scan / pull triggers, connection state, stats. No business changes.

use crate::error::{Result, SyncError};
use crate::events::{EventSubscriber, SyncEvent};
use crate::model::{FolderState, SyncManager, SyncStats};
use crate::service::SyncService;
use syncthing_core::types::{Config, Folder};
use syncthing_core::DeviceId;
use tracing::info;

#[async_trait::async_trait]
impl SyncManager for SyncService {
    async fn get_config(&self) -> Result<Config> {
        Ok(self.config.read().await.clone())
    }

    async fn update_config(&self, config: Config) -> Result<()> {
        *self.config.write().await = config;
        Ok(())
    }

    async fn add_device(&self, device: syncthing_core::types::Device) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.devices.push(device);
        }
        Ok(())
    }

    async fn remove_device(&self, device_id: &DeviceId) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.devices.retain(|d| d.id != *device_id);
        }
        self.connected_devices.remove(device_id);
        Ok(())
    }

    async fn add_folder(&self, folder: Folder) -> Result<()> {
        // 添加到配置
        {
            let mut config = self.config.write().await;
            config.folders.push(folder.clone());
        }

        // 初始化文件夹（创建 FolderModel）
        let folder_id = folder.id.clone();
        self.add_folder_internal(folder).await?;

        // T2.6 fix (KNOWN_ISSUES §2): also spawn scan/pull/watcher loops so that
        // folders added at runtime (via REST API, TUI, or test harness) actually
        // start synchronizing. Previously only `SyncManager::start()` spawned
        // loops, leaving any folder added after startup silently inactive
        // (pull_notify would fire but no task was awaiting it, dropping the
        // signal — the end-to-end sync chain breakage diagnosed in v0.2.4).
        // `start_folder_internal` is idempotent (early-returns if already
        // running), so this is safe to call unconditionally.
        self.start_folder_internal(&folder_id).await?;

        Ok(())
    }

    async fn remove_folder(&self, folder_id: &str) -> Result<()> {
        // 从配置中移除
        {
            let mut config = self.config.write().await;
            config.folders.retain(|f| f.id != folder_id);
        }

        // 从运行时移除
        if self.folders.remove(folder_id).is_some() {
            info!(folder_id = %folder_id, "Folder removed");
        }

        Ok(())
    }

    async fn get_folder_state(&self, folder_id: &str) -> Result<FolderState> {
        match self.folders.get(folder_id) {
            Some(folder) => Ok(folder.state().await),
            None => Err(SyncError::FolderNotFound(folder_id.to_string())),
        }
    }

    async fn start(&self) -> Result<()> {
        info!("Starting sync service");

        // 初始化文件夹
        self.init_folders().await?;

        // 启动文件夹循环
        self.start_folder_loops().await;

        info!("Sync service started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Stopping sync service");

        // 发送全局关闭信号
        self.shutdown_tx.send(true).ok();

        // 收集所有 folder_id
        let folder_ids: Vec<String> = self.folder_tasks.iter().map(|e| e.key().clone()).collect();

        // 发送停止信号
        for folder_id in &folder_ids {
            if let Some(handles) = self.folder_tasks.get(folder_id) {
                handles.shutdown_tx.send(true).ok();
            }
        }

        // 等待所有任务完成
        for folder_id in folder_ids {
            if let Some((_, handles)) = self.folder_tasks.remove(&folder_id) {
                let _ = handles.scan_handle.await;
                let _ = handles.pull_handle.await;
                let _ = handles.watcher_handle.await;
            }
        }

        info!("Sync service stopped");
        Ok(())
    }

    async fn scan_folder(&self, folder_id: &str) -> Result<()> {
        match self.folders.get(folder_id) {
            Some(folder) => {
                folder.scan().await?;
                Ok(())
            }
            None => Err(SyncError::FolderNotFound(folder_id.to_string())),
        }
    }

    async fn scan_folder_sub(&self, folder_id: &str, sub: &str) -> Result<()> {
        match self.folders.get(folder_id) {
            Some(folder) => {
                folder.scan_sub(sub).await?;
                Ok(())
            }
            None => Err(SyncError::FolderNotFound(folder_id.to_string())),
        }
    }

    async fn pull_folder(&self, folder_id: &str) -> Result<()> {
        match self.folders.get(folder_id) {
            Some(folder) => {
                folder.pull().await?;
                Ok(())
            }
            None => Err(SyncError::FolderNotFound(folder_id.to_string())),
        }
    }

    async fn get_connected_devices(&self) -> Result<Vec<DeviceId>> {
        Ok(self
            .connected_devices
            .iter()
            .map(|e: dashmap::mapref::multiple::RefMulti<'_, DeviceId, ()>| *e.key())
            .collect())
    }

    async fn connect_device(&self, device_id: DeviceId) -> Result<()> {
        self.connected_devices.insert(device_id, ());
        self.events
            .publish(SyncEvent::DeviceConnected { device: device_id });
        info!(device = %device_id.short_id(), "Device connected");
        Ok(())
    }

    async fn disconnect_device(&self, device_id: DeviceId) -> Result<()> {
        self.connected_devices.remove(&device_id);
        self.events.publish(SyncEvent::DeviceDisconnected {
            device: device_id,
            reason: "Manual disconnect".to_string(),
        });
        info!(device = %device_id.short_id(), "Device disconnected");
        Ok(())
    }

    fn subscribe_events(&self) -> EventSubscriber {
        self.events.subscribe()
    }

    async fn get_stats(&self) -> Result<SyncStats> {
        let mut stats = SyncStats::default();

        for entry in self.folders.iter() {
            let folder = entry.value();
            let state = folder.state().await;

            if let Ok(files) = self.db.get_folder_files(folder.id()).await {
                let folder_stats = crate::model::FolderStats {
                    files: state.local_files,
                    directories: files
                        .iter()
                        .filter(|f| {
                            matches!(f.file_type, syncthing_core::types::FileType::Directory)
                        })
                        .count(),
                    symlinks: files
                        .iter()
                        .filter(|f| matches!(f.file_type, syncthing_core::types::FileType::Symlink))
                        .count(),
                    total_bytes: files.iter().map(|f| f.size as u64).sum(),
                    deleted: files.iter().filter(|f| f.is_deleted()).count(),
                };

                let files_count = folder_stats.files;
                let bytes_count = folder_stats.total_bytes;
                stats.folders.insert(folder.id().to_string(), folder_stats);
                stats.total_files += files_count;
                stats.total_bytes += bytes_count;
            }
        }

        Ok(stats)
    }
}
