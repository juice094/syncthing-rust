//! T2.5/T2.6 - End-to-end sync test using the BEP bridge in TestNode harness.
//!
//! Status (2026-05-13 post-T2.6): **PASSING**.
//!
//! This test was introduced by T2.5 as a diagnostic that originally exposed a
//! real end-to-end sync defect: `SyncManager::add_folder` created the
//! `FolderModel` but never spawned its scan/pull/watcher tasks, leaving any
//! folder added at runtime silently inactive. The `pull_notify.notify_one()`
//! from `handle_remote_index` was being delivered to a Notify with no
//! awaiter, so the block-request chain was never driven.
//!
//! T2.6 fix (one-line addition in `service::add_folder`): unconditionally
//! call `start_folder_internal` after `add_folder_internal`. The helper is
//! idempotent. See `docs/KNOWN_ISSUES.md` §2 for the full investigation.
//!
//! Verification surface (in execution order):
//!
//! 1. TLS handshake + Hello exchange
//! 2. ClusterConfig exchange
//! 3. Index sync (sender broadcasts file metadata)
//! 4. Block transfer (receiver requests blocks via BEP)
//! 5. File materialization on receiver

use std::time::Duration;
use syncthing_core::types::Folder;
use syncthing_sync::SyncManager;
use syncthing_test_utils::harness::TestNode;

#[tokio::test]
#[serial_test::serial]
async fn test_two_node_single_file_sync() {
    // Init tracing for debugging (no-op if already initialized).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init();

    let node_a = TestNode::new("sync-a").await.expect("create node a");
    let node_b = TestNode::new("sync-b").await.expect("create node b");

    // Create shared folders on both nodes (must use same folder id)
    let folder_a_path = node_a.config_dir.join("sync");
    let folder_b_path = node_b.config_dir.join("sync");

    let mut folder_a = Folder::new("e2e", folder_a_path.to_str().unwrap());
    folder_a.devices.push(node_b.device_id);
    node_a.add_folder(folder_a).await.expect("a add folder");

    let mut folder_b = Folder::new("e2e", folder_b_path.to_str().unwrap());
    folder_b.devices.push(node_a.device_id);
    node_b.add_folder(folder_b).await.expect("b add folder");

    // Drop a small test file on node A *before* connecting (simpler model)
    let test_payload = b"T2.5 end-to-end sync verification - hello from node A!".to_vec();
    let file_path = folder_a_path.join("hello.txt");
    tokio::fs::write(&file_path, &test_payload)
        .await
        .expect("write test file");

    // Force a folder scan on A so the file is picked up by the index before
    // any BEP Index message is sent. Otherwise the first Index would be empty
    // and we would have to wait for the periodic rescan (default 3600s).
    node_a
        .sync_service
        .scan_folder("e2e")
        .await
        .expect("a scan folder");

    // Initiate connections in both directions
    node_a.connect_to(&node_b).await.expect("a connect to b");
    node_b.connect_to(&node_a).await.expect("b connect to a");

    // Wait for connections to establish (covered by handshake test already)
    node_a
        .wait_for_connection(node_b.device_id, Duration::from_secs(15))
        .await
        .expect("a wait for b");
    node_b
        .wait_for_connection(node_a.device_id, Duration::from_secs(15))
        .await
        .expect("b wait for a");

    // T2.5: Wait for the file to appear on node B (Index + Block pull).
    // Generous timeout accounts for: initial ClusterConfig 10s timeout +
    // reconnect backoff 1-3s + index propagation + block transfer.
    // NOTE: under `cargo test --workspace` parallelism, CPU contention can
    // delay the sync pipeline; 90s is a safe upper bound (isolated run ~12s).
    let file_b = folder_b_path.join("hello.txt");
    let start = std::time::Instant::now();
    let mut found = false;
    while start.elapsed() < Duration::from_secs(90) {
        if file_b.exists() {
            let received = tokio::fs::read(&file_b).await.expect("read file_b");
            if received == test_payload {
                found = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    node_a.shutdown().await;
    node_b.shutdown().await;

    assert!(
        found,
        "T2.5: file did not sync to node B within 90s - BEP bridge not driving full pipeline?"
    );
}
