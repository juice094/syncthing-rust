use super::*;
use crate::database::MemoryDatabase;
use crate::puller::BlockSource;
use bytes::Bytes;
use syncthing_core::types::BlockInfo;

struct MockBlockSource {
    data: Bytes,
}

#[async_trait::async_trait]
impl BlockSource for MockBlockSource {
    async fn request_block(
        &self,
        _folder: &str,
        _file: &str,
        _block: &BlockInfo,
        _block_no: usize,
    ) -> crate::error::Result<Bytes> {
        Ok(self.data.clone())
    }
}

#[tokio::test]
async fn test_folder_model_creation() {
    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);
    let folder = Folder::new("test", "/tmp/test");

    let model = FolderModel::new(folder, db, events, None);
    assert_eq!(model.id(), "test");
}

/// 验证 handle_remote_index 能唤醒 pull loop，并触发文件下载
#[tokio::test]
async fn test_pull_notify_wakeup() {
    use sha2::Digest;

    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);

    let temp_dir = tempfile::tempdir().unwrap();
    let folder_path = temp_dir.path().to_path_buf();
    let folder = Folder::new("test-folder", folder_path.to_str().unwrap());

    // 准备测试数据
    let test_data = b"notify pull test";
    let hash = sha2::Sha256::digest(test_data);
    let file_info = FileInfo {
        name: "notify_test.txt".to_string(),
        file_type: syncthing_core::types::FileType::File,
        size: test_data.len() as i64,
        permissions: 0o644,
        modified_s: 0,
        modified_ns: 0,
        version: syncthing_core::types::Vector::new(),
        sequence: 1,
        block_size: test_data.len() as i32,
        blocks: vec![BlockInfo {
            size: test_data.len() as i32,
            hash: hash.to_vec(),
            offset: 0,
        }],
        symlink_target: None,
        deleted: Some(false),
        modified_by: None,
        blocks_hash: None,
        no_permissions: None,
        base_version: None,
    };

    // 模拟远程索引已更新到 DB
    db.update_file(&folder.id, file_info).await.unwrap();

    let mock_source = std::sync::Arc::new(MockBlockSource {
        data: Bytes::from_static(test_data),
    });
    let model = std::sync::Arc::new(FolderModel::new(
        folder,
        db.clone(),
        events,
        Some(mock_source),
    ));

    // 启动 pull loop
    let (tx, rx) = tokio::sync::watch::channel(false);
    let model_clone = std::sync::Arc::clone(&model);
    let handle = tokio::spawn(async move {
        model_clone.start_pull_loop(rx).await;
    });

    // 调用 handle_remote_index 唤醒 pull loop
    let device = syncthing_core::DeviceId::default();
    model.handle_remote_index(device, vec![]).await.unwrap();

    // 等待 pull loop 执行（最多 5 秒）
    tokio::time::timeout(tokio::time::Duration::from_secs(5), async {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let file_path = folder_path.join("notify_test.txt");
            if file_path.exists() {
                break;
            }
        }
    })
    .await
    .unwrap();

    // 验证文件被下载
    let file_path = folder_path.join("notify_test.txt");
    assert!(file_path.exists(), "File should be pulled after notify");
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert_eq!(content, "notify pull test");

    // 停止 pull loop
    tx.send(true).unwrap();
    handle.await.unwrap();
}

/// 验证增量扫描能只发现新增文件，而不重新全量扫描
#[tokio::test]
async fn test_scan_incremental_new_file() {
    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);

    let temp_dir = tempfile::tempdir().unwrap();
    let folder_path = temp_dir.path().to_path_buf();
    let folder = Folder::new("test-folder", folder_path.to_str().unwrap());

    // 先全量扫描建立 baseline
    let model = FolderModel::new(folder.clone(), db.clone(), events.clone(), None);
    model.scan().await.unwrap();

    // 新增一个文件
    let new_file = folder_path.join("new.txt");
    tokio::fs::write(&new_file, "hello").await.unwrap();

    // 只扫描 new.txt 所在的子树
    let changed = model
        .scan_incremental(vec!["new.txt".to_string()], vec![])
        .await
        .unwrap();

    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].name, "new.txt");

    // DB 中应包含该文件
    let db_file = db.get_file(&folder.id, "new.txt").await.unwrap();
    assert!(db_file.is_some());
}

/// 验证增量扫描能处理显式删除事件
#[tokio::test]
async fn test_scan_incremental_delete() {
    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);

    let temp_dir = tempfile::tempdir().unwrap();
    let folder_path = temp_dir.path().to_path_buf();
    let folder = Folder::new("test-folder", folder_path.to_str().unwrap());

    // 预置文件并全量扫描
    let existing = folder_path.join("old.txt");
    tokio::fs::write(&existing, "content").await.unwrap();
    let model = FolderModel::new(folder.clone(), db.clone(), events.clone(), None);
    model.scan().await.unwrap();
    assert!(db.get_file(&folder.id, "old.txt").await.unwrap().is_some());

    // 删除文件
    tokio::fs::remove_file(&existing).await.unwrap();

    // 增量扫描只处理删除路径
    let changed = model
        .scan_incremental(vec![], vec!["old.txt".to_string()])
        .await
        .unwrap();

    assert_eq!(changed.len(), 1);
    assert!(changed[0].is_deleted());

    let db_file = db.get_file(&folder.id, "old.txt").await.unwrap().unwrap();
    assert!(db_file.is_deleted());
}

/// 验证脏路径超过阈值时回退到全量扫描
#[tokio::test]
async fn test_dirty_set_fallback_to_full() {
    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);

    let temp_dir = tempfile::tempdir().unwrap();
    let folder_path = temp_dir.path().to_path_buf();
    let folder = Folder::new("test-folder", folder_path.to_str().unwrap());

    let model = FolderModel::new(folder.clone(), db.clone(), events.clone(), None);

    // 直接写入大量脏路径，超过默认阈值（max(100, local_files/10) = 100）
    for i in 0..150usize {
        model.dirty_changes.insert(format!("file_{}.txt", i));
    }

    let changed = model.process_dirty_set().await.unwrap();
    // 全量扫描空目录，没有变更
    assert!(changed.is_empty());
    // 脏集合应已被清空
    assert!(model.dirty_changes.is_empty());
}
