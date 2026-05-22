use crate::kv::SledStore;
use crate::metadata::MetadataStore;
use syncthing_core::{FileInfo, FolderId};
use tempfile::TempDir;

fn create_test_file_info(name: &str, size: u64) -> FileInfo {
    FileInfo {
        name: name.to_string(),
        file_type: syncthing_core::FileType::File,
        size: size as i64,
        permissions: 0o644,
        modified_s: 0,
        modified_ns: 0,
        version: syncthing_core::Vector::new(),
        sequence: 0,
        block_size: 0,
        blocks: vec![],
        symlink_target: None,
        deleted: Some(false),
        modified_by: None,
        blocks_hash: None,
        no_permissions: None,
    }
}

#[tokio::test]
async fn test_metadata_store_basic() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = MetadataStore::new(sled_store);

    let folder = FolderId::new("test-folder");
    let info = create_test_file_info("test.txt", 1024);

    // Test put and get
    store.put_file(&folder, &info).await.unwrap();
    let retrieved = store.get_file(&folder, "test.txt").await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().name, "test.txt");

    // Test non-existent file
    let missing = store.get_file(&folder, "missing.txt").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn test_folder_index() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = MetadataStore::new(sled_store);

    let folder = FolderId::new("test-folder");

    // Add multiple files
    store
        .put_file(&folder, &create_test_file_info("file1.txt", 100))
        .await
        .unwrap();
    store
        .put_file(&folder, &create_test_file_info("file2.txt", 200))
        .await
        .unwrap();
    store
        .put_file(&folder, &create_test_file_info("file3.txt", 300))
        .await
        .unwrap();

    // Get folder index
    let index = store.get_folder_index(&folder).await.unwrap();
    assert_eq!(index.len(), 3);

    // Test file count
    let count = store.file_count(&folder).await.unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_update_index() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = MetadataStore::new(sled_store);

    let folder = FolderId::new("test-folder");

    // Add initial files
    store
        .put_file(&folder, &create_test_file_info("file1.txt", 100))
        .await
        .unwrap();
    store
        .put_file(&folder, &create_test_file_info("file2.txt", 200))
        .await
        .unwrap();

    // Replace with new index
    let new_index = vec![
        create_test_file_info("new1.txt", 1000),
        create_test_file_info("new2.txt", 2000),
        create_test_file_info("new3.txt", 3000),
    ];

    store.update_index(&folder, new_index).await.unwrap();

    let index = store.get_folder_index(&folder).await.unwrap();
    assert_eq!(index.len(), 3);

    // Old files should be gone
    assert!(store
        .get_file(&folder, "file1.txt")
        .await
        .unwrap()
        .is_none());
    assert!(store
        .get_file(&folder, "file2.txt")
        .await
        .unwrap()
        .is_none());

    // New files should exist
    assert!(store.get_file(&folder, "new1.txt").await.unwrap().is_some());
    assert!(store.get_file(&folder, "new2.txt").await.unwrap().is_some());
    assert!(store.get_file(&folder, "new3.txt").await.unwrap().is_some());
}

#[tokio::test]
async fn test_update_index_delta() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = MetadataStore::new(sled_store);

    let folder = FolderId::new("test-folder");

    // Add initial files
    store
        .put_file(&folder, &create_test_file_info("file1.txt", 100))
        .await
        .unwrap();
    store
        .put_file(&folder, &create_test_file_info("file2.txt", 200))
        .await
        .unwrap();

    // Update with delta
    let delta = vec![
        create_test_file_info("file2.txt", 250), // Update
        create_test_file_info("file3.txt", 300), // Add
    ];

    store.update_index_delta(&folder, delta).await.unwrap();

    let index = store.get_folder_index(&folder).await.unwrap();
    assert_eq!(index.len(), 3);

    // file1.txt should still exist
    assert!(store
        .get_file(&folder, "file1.txt")
        .await
        .unwrap()
        .is_some());

    // file2.txt should be updated
    let file2 = store.get_file(&folder, "file2.txt").await.unwrap().unwrap();
    assert_eq!(file2.size, 250);

    // file3.txt should be added
    assert!(store
        .get_file(&folder, "file3.txt")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn test_deleted_file() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = MetadataStore::new(sled_store);

    let folder = FolderId::new("test-folder");

    // Add a file
    let mut info = create_test_file_info("deleted.txt", 100);
    store.put_file(&folder, &info).await.unwrap();

    // Mark as deleted and update
    info.mark_deleted();
    store.update_index_delta(&folder, vec![info]).await.unwrap();

    // The file should still be in the index (for sync purposes)
    // but marked as deleted
    let retrieved = store.get_file(&folder, "deleted.txt").await.unwrap();
    assert!(retrieved.is_some());
    assert!(retrieved.unwrap().is_deleted());
}

#[tokio::test]
async fn test_folder_stats() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = MetadataStore::new(sled_store);

    let folder = FolderId::new("test-folder");

    // Initially empty
    let stats = store.get_folder_stats(&folder).await.unwrap();
    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.total_bytes, 0);

    // Add files
    store
        .put_file(&folder, &create_test_file_info("file1.txt", 100))
        .await
        .unwrap();
    store
        .put_file(&folder, &create_test_file_info("file2.txt", 200))
        .await
        .unwrap();

    // Stats should be updated
    let stats = store.get_folder_stats(&folder).await.unwrap();
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.total_bytes, 300);
}

#[tokio::test]
async fn test_multiple_folders() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = MetadataStore::new(sled_store);

    let folder1 = FolderId::new("folder1");
    let folder2 = FolderId::new("folder2");

    store
        .put_file(&folder1, &create_test_file_info("file.txt", 100))
        .await
        .unwrap();
    store
        .put_file(&folder2, &create_test_file_info("file.txt", 200))
        .await
        .unwrap();

    // Files should be isolated by folder
    let file1 = store.get_file(&folder1, "file.txt").await.unwrap().unwrap();
    let file2 = store.get_file(&folder2, "file.txt").await.unwrap().unwrap();

    assert_eq!(file1.size, 100);
    assert_eq!(file2.size, 200);
}

#[tokio::test]
async fn test_device_files() {
    let temp_dir = TempDir::new().unwrap();
    let sled_store = SledStore::open(temp_dir.path()).unwrap();
    let store = MetadataStore::new(sled_store);

    let folder = FolderId::new("test-folder");
    let device_id = "ABC123";

    // Store file for device
    let info = create_test_file_info("device_file.txt", 500);
    store
        .put_device_file(device_id, &folder, &info)
        .await
        .unwrap();

    // Retrieve file for device
    let retrieved = store
        .get_device_file(device_id, &folder, "device_file.txt")
        .await
        .unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().size, 500);

    // Different device should not see the file
    let other = store
        .get_device_file("OTHER", &folder, "device_file.txt")
        .await
        .unwrap();
    assert!(other.is_none());
}
