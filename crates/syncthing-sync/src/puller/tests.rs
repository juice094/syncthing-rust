use super::*;
use crate::database::MemoryDatabase;
use crate::scanner::Scanner;
use std::path::Path;
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
    ) -> Result<Bytes> {
        Ok(self.data.clone())
    }
}

#[tokio::test]
async fn test_puller_creation() {
    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);
    let puller = Puller::new(db, events);
    // 基本创建测试
    assert!(puller.block_source.is_none());
}

#[tokio::test]
async fn test_download_file_with_mock_source() {
    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);

    // 创建临时目录
    let temp_dir = tempfile::tempdir().unwrap();
    let folder_path = temp_dir.path().to_path_buf();

    // 准备测试数据
    let test_data = b"hello world";
    let hash = sha2::Sha256::digest(test_data);

    let file_info = FileInfo {
        name: "test.txt".to_string(),
        file_type: syncthing_core::types::FileType::File,
        size: test_data.len() as i64,
        permissions: 0o644,
        modified_s: 0,
        modified_ns: 0,
        version: syncthing_core::types::Vector::new(),
        sequence: 0,
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

    let mock_source = Arc::new(MockBlockSource {
        data: Bytes::from_static(test_data),
    });

    let result = Puller::download_file(
        &folder_path,
        &file_info,
        &*db,
        &events,
        "test-folder",
        Some(mock_source),
        16,
        None,
    )
    .await;

    assert!(result.is_ok());

    // 验证文件内容
    let file_path = folder_path.join("test.txt");
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert_eq!(content, "hello world");
}

/// 集成测试：模拟远程索引更新后，check_needed_files 能发现 needed 文件，
/// 且 pull_folder 能成功下载。
#[tokio::test]
async fn test_check_needed_files_then_pull() {
    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);

    // 创建临时目录作为 folder path
    let temp_dir = tempfile::tempdir().unwrap();
    let folder_path = temp_dir.path().to_path_buf();
    let folder = syncthing_core::types::Folder::new("test-folder", folder_path.to_str().unwrap());

    // 准备测试数据
    let test_data = b"pull test content";
    let hash = sha2::Sha256::digest(test_data);

    let file_info = FileInfo {
        name: "pull_test.txt".to_string(),
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

    // 模拟 index_handler 处理远程索引后更新 DB
    db.update_file(&folder.id, file_info.clone()).await.unwrap();

    // 创建 Puller + MockBlockSource
    let mock_source = Arc::new(MockBlockSource {
        data: Bytes::from_static(test_data),
    });
    let puller = Puller::new(db.clone(), events.clone()).with_block_source(Some(mock_source));

    // Step 1: check_needed_files 应该发现本地不存在的文件
    let needed = puller.check_needed_files(&folder).await.unwrap();
    assert_eq!(needed.len(), 1, "Should detect 1 needed file");
    assert_eq!(needed[0].name, "pull_test.txt");

    // Step 2: pull_folder 应该成功下载文件
    let stats = puller.pull_folder(&folder, needed).await.unwrap();
    assert_eq!(stats.files_succeeded, 1, "Should succeed pulling 1 file");
    assert_eq!(stats.files_failed, 0);

    // Step 3: 验证本地文件内容正确
    let file_path = folder_path.join("pull_test.txt");
    assert!(file_path.exists(), "File should exist after pull");
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert_eq!(content, "pull test content");

    // Step 4: 再次 check_needed_files，应该为空（文件已存在且大小匹配）
    let needed_after = puller.check_needed_files(&folder).await.unwrap();
    assert!(
        needed_after.is_empty(),
        "Should not need pull after file exists"
    );
}

/// E2E 集成测试：模拟两节点通过 block_server 同步单文件
/// - 节点 A 扫描本地文件生成索引
/// - 节点 B 接收索引，通过 block_server 从节点 A 读取块，完成 pull
#[tokio::test]
async fn test_e2e_sync_single_file_via_block_server() {
    use crate::index_handler::IndexHandler;
    use std::path::PathBuf;
    use syncthing_core::types::{Folder, Index};

    // 节点 A：创建临时文件并扫描
    let temp_a = tempfile::tempdir().unwrap();
    let file_path = temp_a.path().join("test.txt");
    tokio::fs::write(&file_path, "hello sync world")
        .await
        .unwrap();

    let db_a = MemoryDatabase::new();
    let events_a = EventPublisher::new(10);
    let scanner_a = Scanner::new(db_a.clone(), events_a.clone());
    let folder_a = Folder::new("test", temp_a.path().to_str().unwrap());
    let changed = scanner_a.scan_folder(&folder_a).await.unwrap();
    assert_eq!(changed.len(), 1, "Should detect 1 changed file");
    let file_info = changed.into_iter().next().unwrap();

    // 节点 B：准备接收同步
    let temp_b = tempfile::tempdir().unwrap();
    let db_b = MemoryDatabase::new();
    let events_b = EventPublisher::new(10);

    // BlockSource 通过 block_server 从节点 A 读取块数据
    struct LocalBlockSource {
        folder_root: PathBuf,
    }
    #[async_trait::async_trait]
    impl BlockSource for LocalBlockSource {
        async fn request_block(
            &self,
            _folder: &str,
            file: &str,
            block: &BlockInfo,
            block_no: usize,
        ) -> Result<Bytes> {
            let req = bep_protocol::messages::Request {
                id: 1,
                folder: "test".to_string(),
                name: file.to_string(),
                offset: block.offset,
                size: block.size,
                hash: block.hash.clone(),
                from_temporary: false,
                block_no: block_no as i32,
            };
            let data = crate::block_server::serve_block_request(&self.folder_root, &req)
                .await
                .map_err(|e| SyncError::pull(file.to_string(), e.to_string()))?;
            Ok(Bytes::from(data))
        }
    }

    let block_source = Arc::new(LocalBlockSource {
        folder_root: temp_a.path().to_path_buf(),
    });
    let puller = Puller::new(db_b.clone(), events_b.clone()).with_block_source(Some(block_source));

    // 节点 B 的 index_handler 处理节点 A 的索引
    let index_handler = IndexHandler::new(db_b.clone(), events_b.clone());
    let folder_b = Folder::new("test", temp_b.path().to_str().unwrap());
    let device_a = syncthing_core::DeviceId::random();
    let index = Index {
        folder: "test".to_string(),
        files: vec![file_info],
    };
    let needed = index_handler
        .handle_index(&folder_b, device_a, index)
        .await
        .unwrap();

    // 执行 pull
    let stats = puller.pull_folder(&folder_b, needed).await.unwrap();
    assert_eq!(stats.files_succeeded, 1, "Should pull 1 file");
    assert_eq!(stats.files_failed, 0);

    // 验证节点 B 本地文件内容
    let dest = temp_b.path().join("test.txt");
    assert!(dest.exists(), "File should exist after pull");
    let content = tokio::fs::read_to_string(&dest).await.unwrap();
    assert_eq!(content, "hello sync world");

    // 验证数据库已更新
    let db_file = db_b.get_file("test", "test.txt").await.unwrap();
    assert!(db_file.is_some(), "File should be in database after pull");
}

// ── temp_path_for / rename_with_retry 单元测试 ──

#[test]
fn test_temp_path_for_basic() {
    let path = Path::new("/folder/sub/file.md");
    let temp = temp_path_for(path);
    assert_eq!(temp, Path::new("/folder/sub/.syncthing.file.md.tmp"));
}

#[test]
fn test_temp_path_for_no_parent() {
    let path = Path::new("file.txt");
    let temp = temp_path_for(path);
    assert_eq!(temp, Path::new(".syncthing.file.txt.tmp"));
}

#[test]
fn test_temp_path_for_unicode_filename() {
    let path = Path::new("/data/中文文件.txt");
    let temp = temp_path_for(path);
    assert_eq!(temp, Path::new("/data/.syncthing.中文文件.txt.tmp"));
}

#[test]
fn test_temp_path_for_deeply_nested() {
    let path = Path::new("/a/b/c/d/file.txt");
    let temp = temp_path_for(path);
    assert_eq!(temp, Path::new("/a/b/c/d/.syncthing.file.txt.tmp"));
}

/// 验证 rename_with_retry 在正常情况下成功
#[tokio::test]
async fn test_rename_with_retry_success() {
    let dir = tempfile::tempdir().unwrap();
    let temp_path = dir.path().join(".syncthing.test.txt.tmp");
    let real_path = dir.path().join("test.txt");

    // 创建 temp 文件
    tokio::fs::write(&temp_path, b"hello rename").await.unwrap();

    let result = rename_with_retry(&temp_path, &real_path, "test.txt").await;
    assert!(result.is_ok(), "Rename should succeed: {:?}", result.err());
    assert!(real_path.exists(), "Real file should exist after rename");
    assert!(
        !temp_path.exists(),
        "Temp file should not exist after rename"
    );

    let content = tokio::fs::read_to_string(&real_path).await.unwrap();
    assert_eq!(content, "hello rename");
}

/// 验证 rename_with_retry 在目标已存在时仍能成功（remove fallback）
#[tokio::test]
async fn test_rename_with_retry_target_exists() {
    let dir = tempfile::tempdir().unwrap();
    let temp_path = dir.path().join(".syncthing.test.txt.tmp");
    let real_path = dir.path().join("test.txt");

    // 目标文件先存在（模拟已有文件）
    tokio::fs::write(&real_path, b"old content").await.unwrap();
    // temp 文件是新的
    tokio::fs::write(&temp_path, b"new content").await.unwrap();

    let result = rename_with_retry(&temp_path, &real_path, "test.txt").await;
    assert!(
        result.is_ok(),
        "Rename with target removal should succeed: {:?}",
        result.err()
    );
    assert!(real_path.exists());
    assert!(!temp_path.exists());

    let content = tokio::fs::read_to_string(&real_path).await.unwrap();
    assert_eq!(content, "new content");
}

/// 验证 temp_path_for 生成的是父目录下的隐藏文件，而非修改扩展名
#[test]
fn test_temp_path_preserves_original_extension() {
    // 重要：这验证了 scanner.rs 修复的正确性
    // with_extension(".syncthing.tmp") 会产生 "foo.syncthing.tmp"
    // temp_path_for 产生 ".syncthing.foo.md.tmp"
    let path = Path::new("/dir/foo.md");
    let temp = temp_path_for(path);

    // 验证不是 with_extension 的错误行为
    assert_ne!(temp, Path::new("/dir/foo.syncthing.tmp"));
    // 验证是正确的格式
    assert_eq!(temp, Path::new("/dir/.syncthing.foo.md.tmp"));
}

/// 集成测试：Puller 在下载冲突的远程文本文件时执行三路合并
#[tokio::test]
async fn test_puller_three_way_merge_on_conflict() {
    use syncthing_versioner::create_versioner;

    let db = MemoryDatabase::new();
    let events = EventPublisher::new(10);

    let temp_dir = tempfile::tempdir().unwrap();
    let folder_path = temp_dir.path().to_path_buf();
    let folder = syncthing_core::types::Folder::new("test-folder", folder_path.to_str().unwrap());

    let file_name = "merge_test.md";
    let file_path = folder_path.join(file_name);

    // Step 1: 创建 base 版本并扫描，生成 base_version
    let base_content = "line1\nline2\n";
    tokio::fs::write(&file_path, base_content).await.unwrap();
    let scanner = Scanner::new(db.clone(), events.clone());
    let mut changed = scanner.scan_folder(&folder).await.unwrap();
    assert_eq!(changed.len(), 1);
    let mut local_info = changed.pop().unwrap();
    assert!(
        local_info.base_version.is_some(),
        "Scanner should set base_version"
    );

    // Step 2: 模拟本地修改
    let local_content = "line1\nlocal-add\nline2\n";
    tokio::fs::write(&file_path, local_content).await.unwrap();
    local_info.version.increment(1);
    local_info.sequence = 2;
    db.update_file(&folder.id, local_info.clone())
        .await
        .unwrap();

    // Step 3: 归档 base 版本到 .stversions/
    let versioning_config = syncthing_core::types::VersioningConfig::Simple {
        params: std::collections::HashMap::from([("keep".to_string(), "5".to_string())]),
    };
    let versioner =
        create_versioner(&versioning_config, &folder_path).expect("versioner should be created");
    // 先把 base 版本写回文件再归档
    tokio::fs::write(&file_path, base_content).await.unwrap();
    versioner.archive(&file_path).await.unwrap();
    // 验证归档内容
    let stversions = folder_path.join(".stversions");
    for entry in std::fs::read_dir(&stversions).unwrap() {
        let path = entry.unwrap().path();
        let content = std::fs::read_to_string(&path).unwrap();
        eprintln!("archived {}: {:?}", path.display(), content);
    }
    // 重新写本地修改
    tokio::fs::write(&file_path, local_content).await.unwrap();

    // Step 4: 准备 remote 版本（与本地冲突）
    let remote_content = "line1\nline2\nremote-add\n";
    let hash = sha2::Sha256::digest(remote_content);
    let remote_info = FileInfo {
        name: file_name.to_string(),
        file_type: syncthing_core::types::FileType::File,
        size: remote_content.len() as i64,
        permissions: 0o644,
        modified_s: 1,
        modified_ns: 0,
        version: syncthing_core::types::Vector::new().with_counter(2, 1),
        sequence: 3,
        block_size: remote_content.len() as i32,
        blocks: vec![BlockInfo {
            size: remote_content.len() as i32,
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

    // 模拟 index_handler：检测到冲突后接受 remote，但保留本地 base_version
    let mut db_info = remote_info.clone();
    db_info.base_version = local_info.base_version.clone();
    db.update_file(&folder.id, db_info).await.unwrap();

    // Step 5: Puller 下载 remote 并触发三路合并
    let mock_source = Arc::new(MockBlockSource {
        data: Bytes::from_static(remote_content.as_bytes()),
    });
    let versioner_arc = Arc::from(versioner);
    let puller = Puller::new(db.clone(), events.clone())
        .with_block_source(Some(mock_source))
        .with_versioner(Some(versioner_arc));
    let stats = puller
        .pull_folder(&folder, vec![remote_info.clone()])
        .await
        .unwrap();
    assert_eq!(stats.files_succeeded, 1);

    // Step 6: 验证合并结果
    let merged_content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let db_file = db.get_file(&folder.id, file_name).await.unwrap().unwrap();
    eprintln!("merged content:\n{}", merged_content);
    eprintln!("db base_version: {:?}", db_file.base_version);
    eprintln!("local_info base_version: {:?}", local_info.base_version);
    eprintln!("remote_info base_version: {:?}", remote_info.base_version);
    assert!(
        merged_content.contains("local-add"),
        "Merged content should contain local addition"
    );
    assert!(
        merged_content.contains("remote-add"),
        "Merged content should contain remote addition"
    );
    assert!(
        !merged_content.contains("<<<<<<<"),
        "Non-overlapping edits should not conflict"
    );

    // Step 7: 验证数据库 base_version 已更新
    let db_file = db.get_file(&folder.id, file_name).await.unwrap().unwrap();
    assert!(db_file.base_version.is_some());
}
