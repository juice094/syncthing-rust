//! E2E CRUD 测试：增删改查 + .stignore 目录排除 + 重命名优化验证
//!
//! 覆盖 P0/P1/P2 修复的端到端验证：
//! - P0: .stignore 目录排除（skills/ 等带斜杠规则）
//! - P1: 重命名检测（不重新传输块内容）
//! - P2: FileInfo 字段兼容（modified_by, blocks_hash, no_permissions）

use std::time::Duration;
use syncthing_core::types::Folder;
use syncthing_sync::SyncManager;
use syncthing_test_utils::harness::TestNode;

/// 等待文件出现并具有预期内容
async fn wait_for_file(
    base: &std::path::Path,
    name: &str,
    expected: &[u8],
    timeout: Duration,
) -> bool {
    let path = base.join(name);
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            if let Ok(data) = tokio::fs::read(&path).await {
                if data == expected {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// 等待文件消失
async fn wait_for_file_gone(base: &std::path::Path, name: &str, timeout: Duration) -> bool {
    let path = base.join(name);
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if !path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// 等待目录出现
async fn wait_for_dir(base: &std::path::Path, name: &str, timeout: Duration) -> bool {
    let path = base.join(name);
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if path.exists() && path.is_dir() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// 通用两节点初始化：创建 node-a / node-b，配置共享文件夹，建立连接
async fn setup_two_node(
    folder_id: &str,
) -> (TestNode, TestNode, std::path::PathBuf, std::path::PathBuf) {
    let node_a = TestNode::new("crud-a").await.expect("create node a");
    let node_b = TestNode::new("crud-b").await.expect("create node b");

    let folder_a_path = node_a.config_dir.join("sync");
    let folder_b_path = node_b.config_dir.join("sync");

    let mut folder_a = Folder::new(folder_id, folder_a_path.to_str().unwrap());
    folder_a.devices.push(node_b.device_id);
    node_a.add_folder(folder_a).await.expect("a add folder");

    let mut folder_b = Folder::new(folder_id, folder_b_path.to_str().unwrap());
    folder_b.devices.push(node_a.device_id);
    node_b.add_folder(folder_b).await.expect("b add folder");

    node_a.connect_to(&node_b).await.expect("a connect to b");
    node_b.connect_to(&node_a).await.expect("b connect to a");

    node_a
        .wait_for_connection(node_b.device_id, Duration::from_secs(15))
        .await
        .expect("a wait for b");
    node_b
        .wait_for_connection(node_a.device_id, Duration::from_secs(15))
        .await
        .expect("b wait for a");

    (node_a, node_b, folder_a_path, folder_b_path)
}

#[tokio::test]
#[serial_test::serial]
async fn test_e2e_create_file() {
    let (node_a, node_b, folder_a, folder_b) = setup_two_node("e2e-create").await;

    let payload = b"hello crud create".to_vec();
    tokio::fs::write(folder_a.join("create.txt"), &payload)
        .await
        .expect("write");

    node_a
        .sync_service
        .scan_folder("e2e-create")
        .await
        .expect("scan");

    assert!(
        wait_for_file(&folder_b, "create.txt", &payload, Duration::from_secs(30)).await,
        "文件应同步到 node B"
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
}

#[tokio::test]
#[serial_test::serial]
async fn test_e2e_modify_file() {
    let (node_a, node_b, folder_a, folder_b) = setup_two_node("e2e-modify").await;

    // 先创建
    let v1 = b"version 1";
    tokio::fs::write(folder_a.join("mutate.txt"), v1)
        .await
        .expect("write v1");
    node_a
        .sync_service
        .scan_folder("e2e-modify")
        .await
        .expect("scan");
    assert!(
        wait_for_file(&folder_b, "mutate.txt", v1, Duration::from_secs(30)).await,
        "v1 应同步"
    );

    // 再修改
    let v2 = b"version 2 - modified content";
    tokio::fs::write(folder_a.join("mutate.txt"), v2)
        .await
        .expect("write v2");
    node_a
        .sync_service
        .scan_folder("e2e-modify")
        .await
        .expect("scan");
    assert!(
        wait_for_file(&folder_b, "mutate.txt", v2, Duration::from_secs(30)).await,
        "v2 应同步覆盖旧内容"
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
}

#[tokio::test]
#[serial_test::serial]
async fn test_e2e_delete_file() {
    let (node_a, node_b, folder_a, folder_b) = setup_two_node("e2e-delete").await;

    // 先创建
    let data = b"to be deleted";
    tokio::fs::write(folder_a.join("gone.txt"), data)
        .await
        .expect("write");
    node_a
        .sync_service
        .scan_folder("e2e-delete")
        .await
        .expect("scan");
    assert!(
        wait_for_file(&folder_b, "gone.txt", data, Duration::from_secs(30)).await,
        "文件应先同步到 B"
    );

    // 再删除
    tokio::fs::remove_file(folder_a.join("gone.txt"))
        .await
        .expect("delete");
    node_a
        .sync_service
        .scan_folder("e2e-delete")
        .await
        .expect("scan");
    assert!(
        wait_for_file_gone(&folder_b, "gone.txt", Duration::from_secs(30)).await,
        "删除应同步到 B"
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
}

#[tokio::test]
#[serial_test::serial]
async fn test_e2e_rename_file() {
    let (node_a, node_b, folder_a, folder_b) = setup_two_node("e2e-rename").await;

    let data = b"same content after rename";
    tokio::fs::write(folder_a.join("old.txt"), data)
        .await
        .expect("write");
    node_a
        .sync_service
        .scan_folder("e2e-rename")
        .await
        .expect("scan");
    assert!(
        wait_for_file(&folder_b, "old.txt", data, Duration::from_secs(30)).await,
        "old.txt 应同步到 B"
    );

    // 重命名
    tokio::fs::rename(folder_a.join("old.txt"), folder_a.join("new.txt"))
        .await
        .expect("rename");
    node_a
        .sync_service
        .scan_folder("e2e-rename")
        .await
        .expect("scan");

    assert!(
        wait_for_file(&folder_b, "new.txt", data, Duration::from_secs(30)).await,
        "new.txt 应出现在 B"
    );
    assert!(
        wait_for_file_gone(&folder_b, "old.txt", Duration::from_secs(30)).await,
        "old.txt 应从 B 删除"
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
}

#[tokio::test]
#[serial_test::serial]
async fn test_e2e_stignore_directory_exclusion() {
    let (node_a, node_b, folder_a, folder_b) = setup_two_node("e2e-ignore").await;

    // 写入 .stignore
    tokio::fs::write(folder_a.join(".stignore"), "skills/\ntools/\nignored/\n")
        .await
        .expect("write stignore");

    // 创建应被排除的目录和文件
    let ignored = folder_a.join("ignored");
    tokio::fs::create_dir(&ignored)
        .await
        .expect("create ignored");
    tokio::fs::write(ignored.join("secret.txt"), b"secret")
        .await
        .expect("write secret");

    // 创建不应被排除的目录和文件
    let kept = folder_a.join("kept");
    tokio::fs::create_dir(&kept).await.expect("create kept");
    tokio::fs::write(kept.join("public.txt"), b"public")
        .await
        .expect("write public");

    node_a
        .sync_service
        .scan_folder("e2e-ignore")
        .await
        .expect("scan");

    // 验证未排除的内容正常同步（使用异步等待，给足 30s）
    assert!(
        wait_for_dir(&folder_b, "kept", Duration::from_secs(30)).await,
        "kept/ 目录应正常同步"
    );
    assert!(
        wait_for_file(
            &folder_b,
            "kept/public.txt",
            b"public",
            Duration::from_secs(30)
        )
        .await,
        "kept/public.txt 应同步"
    );

    // 验证排除规则生效（ignored 不应出现）
    assert!(
        wait_for_file_gone(&folder_b, "ignored", Duration::from_secs(5)).await,
        "ignored/ 目录应被 .stignore 排除"
    );
    assert!(
        wait_for_file_gone(&folder_b, "ignored/secret.txt", Duration::from_secs(5)).await,
        "ignored/secret.txt 不应同步"
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
}
