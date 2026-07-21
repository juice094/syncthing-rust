use super::SyncService;
use crate::database::MemoryDatabase;
use crate::error::SyncError;
use crate::model::SyncManager;
use syncthing_core::types::Folder;

#[tokio::test]
async fn test_service_creation() {
    let db = MemoryDatabase::new();
    let service = SyncService::new(db);

    assert!(service.get_folder_ids().is_empty());
}

#[tokio::test]
async fn test_add_folder() {
    let db = MemoryDatabase::new();
    let service = SyncService::new(db);

    let folder = Folder::new("test", "/tmp/test");
    service.add_folder(folder).await.unwrap();

    assert_eq!(service.get_folder_ids().len(), 1);
}

#[tokio::test]
async fn test_folder_not_found() {
    let db = MemoryDatabase::new();
    let service = SyncService::new(db);

    let result = service.get_folder_state("nonexistent").await;
    assert!(matches!(result, Err(SyncError::FolderNotFound(_))));
}

/// 回归：配置变更必须触发重协商钩子，并携带当前已连接设备列表
///（对齐 Go Syncthing 配置变更后断开重连、重新交换 ClusterConfig 的行为）
#[tokio::test]
async fn test_update_config_fires_renegotiation_hook() {
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};
    use syncthing_core::DeviceId;

    let db = MemoryDatabase::new();
    let service = SyncService::new(db);
    let device_id =
        DeviceId::from_str("YTKWHNG-OT27ZGH-6VVBRIJ-OHOUNWT-DYLJ2NR-TCXUXHI-QDUQR2U-OPLCBQG")
            .expect("device id");
    service.connected_devices.insert(device_id, ());

    let calls: Arc<Mutex<Vec<Vec<DeviceId>>>> = Arc::new(Mutex::new(Vec::new()));
    let calls_clone = Arc::clone(&calls);
    service
        .set_renegotiation_hook(Arc::new(move |devices| {
            calls_clone.lock().expect("lock").push(devices);
        }))
        .await;

    service
        .update_config(syncthing_core::types::Config::new())
        .await
        .unwrap();

    let recorded = calls.lock().expect("lock");
    assert_eq!(recorded.len(), 1, "update_config 必须触发一次重协商");
    assert_eq!(recorded[0], vec![device_id], "钩子必须携带已连接设备");
}

/// 无已连接设备时，配置变更不应触发重协商（无对象可重连）
#[tokio::test]
async fn test_update_config_no_hook_when_no_connected_devices() {
    use std::sync::{Arc, Mutex};
    use syncthing_core::DeviceId;

    let db = MemoryDatabase::new();
    let service = SyncService::new(db);

    let calls: Arc<Mutex<Vec<Vec<DeviceId>>>> = Arc::new(Mutex::new(Vec::new()));
    let calls_clone = Arc::clone(&calls);
    service
        .set_renegotiation_hook(Arc::new(move |devices| {
            calls_clone.lock().expect("lock").push(devices);
        }))
        .await;

    service
        .update_config(syncthing_core::types::Config::new())
        .await
        .unwrap();

    assert!(
        calls.lock().expect("lock").is_empty(),
        "无已连接设备时不应触发重协商"
    );
}

/// 回归：add_folder 之后再 set_block_source 必须对已创建的 FolderModel 生效
///（此前 block_source 仅在 FolderModel 创建时读取，后设置不生效，
///  pull 报 "No block source configured"）
#[tokio::test]
async fn test_set_block_source_after_add_folder() {
    use crate::puller::BlockSource;
    use bytes::Bytes;
    use std::sync::Arc;

    struct StubSource;
    #[async_trait::async_trait]
    impl BlockSource for StubSource {
        async fn request_block(
            &self,
            _folder: &str,
            _file: &str,
            _block: &syncthing_core::types::BlockInfo,
            _block_no: usize,
        ) -> crate::error::Result<Bytes> {
            Ok(Bytes::new())
        }
    }

    let db = MemoryDatabase::new();
    let service = SyncService::new(db);
    service
        .add_folder(Folder::new("test", "/tmp/test"))
        .await
        .unwrap();
    service.set_block_source(Arc::new(StubSource)).await;

    let model = service.folders.get("test").expect("folder model");
    let has_source = model.puller_block_source_present();
    assert!(
        has_source,
        "set_block_source must propagate to existing FolderModels"
    );
}
