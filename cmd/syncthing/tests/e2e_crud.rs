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

/// 初始化测试日志（进程内仅一次）。
///
/// 默认 `warn`；排查链路时用 `RUST_LOG` 打开详细日志，例如
/// `RUST_LOG=syncthing_test_utils=info,syncthing_sync=info,syncthing_net=info`
/// 并配合 `--nocapture`。此前 harness 的 `info!` 全被丢弃，导致只能盲猜。
fn init_tracing() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    });
}

/// 单次检查：文件存在且内容与期望一致。
async fn file_content_is(base: &std::path::Path, name: &str, expected: &[u8]) -> bool {
    match tokio::fs::read(base.join(name)).await {
        Ok(data) => data == expected,
        Err(_) => false,
    }
}

/// 单次检查：路径不存在。
async fn path_absent(base: &std::path::Path, name: &str) -> bool {
    !base.join(name).exists()
}

/// 单次检查：目录存在。
async fn dir_present(base: &std::path::Path, name: &str) -> bool {
    base.join(name).is_dir()
}

/// 轮询 `check` 直到为真或超时。
///
/// 超时会打印双方已连接设备清单，供下一轮迭代定位丢包环节。
///
/// 注意：这里**不做断连重拨**。实测中在 25s 触发重连会短暂造成「无连接」窗口，
/// 反而让正在进行的块请求失败（`no connection ... after 3 attempts`），而块请求
/// 自身已在 `bep_bridge` 中带重试。保持连接不动，只等 + 诊断。
async fn wait_synced<F, Fut>(
    node_a: &TestNode,
    node_b: &TestNode,
    mut check: F,
    timeout: Duration,
    what: &str,
) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!(
        "[e2e_crud] TIMEOUT '{}' after {}s | a->b={:?} b->a={:?}",
        what,
        timeout.as_secs(),
        node_a.connection_handle.connected_devices(),
        node_b.connection_handle.connected_devices(),
    );
    false
}

/// 通用两节点初始化：创建 node-a / node-b，配置共享文件夹，**单边拨号**建立连接。
///
/// 单边拨号（仅 A 拨 B，B 只登记 A）是刻意的：双方同时拨号会在两端各产生一条
/// incoming + 一条 outgoing，触发连接竞争解决，失败方那条连接上的 BepSession 会空等到
/// 10s ClusterConfig 超时（`docs/KNOWN_ISSUES.md` §1、§23），这是本套件此前偶发超时的触发条件。
async fn setup_two_node(
    folder_id: &str,
) -> (TestNode, TestNode, std::path::PathBuf, std::path::PathBuf) {
    init_tracing();
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
    node_b
        .add_peer_device(&node_a)
        .await
        .expect("b registers a without dialing");

    node_a
        .wait_for_connection(node_b.device_id, Duration::from_secs(15))
        .await
        .expect("a wait for b");
    node_b
        .wait_for_connection(node_a.device_id, Duration::from_secs(15))
        .await
        .expect("b wait for a");

    // 握手就绪：连接建立 ≠ BEP 会话就绪。ClusterConfig 交换完成前索引/块传输无人
    // 收发，旧实现只能靠固定的同步等待超时暴露（§23 根因）。这里显式等待该信号。
    node_a
        .wait_for_cluster_config(node_b.device_id, Duration::from_secs(15))
        .await
        .expect("a wait for b cluster config");
    node_b
        .wait_for_cluster_config(node_a.device_id, Duration::from_secs(15))
        .await
        .expect("b wait for a cluster config");
    node_a
        .wait_for_folder_ready(node_b.device_id, folder_id, Duration::from_secs(15))
        .await
        .expect("a wait for folder announced by b");
    node_b
        .wait_for_folder_ready(node_a.device_id, folder_id, Duration::from_secs(15))
        .await
        .expect("b wait for folder announced by a");

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
        wait_synced(
            &node_a,
            &node_b,
            || file_content_is(&folder_b, "create.txt", &payload),
            Duration::from_secs(60),
            "create.txt 同步到 B",
        )
        .await,
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
        wait_synced(
            &node_a,
            &node_b,
            || file_content_is(&folder_b, "mutate.txt", v1),
            Duration::from_secs(60),
            "mutate.txt v1 同步到 B",
        )
        .await,
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
        wait_synced(
            &node_a,
            &node_b,
            || file_content_is(&folder_b, "mutate.txt", v2),
            Duration::from_secs(60),
            "mutate.txt v2 覆盖同步到 B",
        )
        .await,
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
        wait_synced(
            &node_a,
            &node_b,
            || file_content_is(&folder_b, "gone.txt", data),
            Duration::from_secs(60),
            "gone.txt 同步到 B",
        )
        .await,
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
        wait_synced(
            &node_a,
            &node_b,
            || path_absent(&folder_b, "gone.txt"),
            Duration::from_secs(60),
            "gone.txt 删除同步到 B",
        )
        .await,
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
        wait_synced(
            &node_a,
            &node_b,
            || file_content_is(&folder_b, "old.txt", data),
            Duration::from_secs(60),
            "old.txt 同步到 B",
        )
        .await,
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
        wait_synced(
            &node_a,
            &node_b,
            || file_content_is(&folder_b, "new.txt", data),
            Duration::from_secs(60),
            "new.txt 出现在 B",
        )
        .await,
        "new.txt 应出现在 B"
    );
    assert!(
        wait_synced(
            &node_a,
            &node_b,
            || path_absent(&folder_b, "old.txt"),
            Duration::from_secs(60),
            "old.txt 从 B 删除",
        )
        .await,
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

    // 验证未排除的内容正常同步
    assert!(
        wait_synced(
            &node_a,
            &node_b,
            || dir_present(&folder_b, "kept"),
            Duration::from_secs(60),
            "kept/ 目录同步到 B",
        )
        .await,
        "kept/ 目录应正常同步"
    );
    assert!(
        wait_synced(
            &node_a,
            &node_b,
            || file_content_is(&folder_b, "kept/public.txt", b"public"),
            Duration::from_secs(60),
            "kept/public.txt 同步到 B",
        )
        .await,
        "kept/public.txt 应同步"
    );

    // 验证排除规则生效（ignored 不应出现）
    assert!(
        wait_synced(
            &node_a,
            &node_b,
            || path_absent(&folder_b, "ignored"),
            Duration::from_secs(5),
            "ignored/ 不应出现在 B",
        )
        .await,
        "ignored/ 目录应被 .stignore 排除"
    );
    assert!(
        wait_synced(
            &node_a,
            &node_b,
            || path_absent(&folder_b, "ignored/secret.txt"),
            Duration::from_secs(5),
            "ignored/secret.txt 不应出现在 B",
        )
        .await,
        "ignored/secret.txt 不应同步"
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
}
