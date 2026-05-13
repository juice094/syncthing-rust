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
