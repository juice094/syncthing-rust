//! T2.5 - End-to-end sync test using the BEP bridge in TestNode harness.
//!
//! ⚠️ STATUS (2026-05-13): **FAILING by design — pinned with `#[ignore]`.**
//!
//! This test was added by T2.5 as a **diagnostic** that exposes a real
//! end-to-end sync defect in syncthing-rust. The intent is:
//!
//! 1. Prove the BEP bridge wiring is correct (it is — see logs)
//! 2. Drive ClusterConfig + Index exchange (works — see logs)
//! 3. Verify file materialization on receiver (FAILS — see `docs/KNOWN_ISSUES.md`)
//!
//! Observation from running with `RUST_LOG=info,syncthing_sync=debug`:
//! - ClusterConfig is exchanged (after one 10s reconnect cycle)
//! - Sender publishes Index with 1 file (after `scan_folder()` forces a rescan)
//! - Receiver logs `New file from remote file=hello.txt`
//! - **But no subsequent Block request / Response is observed**
//! - File never materializes on receiver within 45s
//!
//! Root cause (suspected): index_handler → puller → BlockSource::request_block
//! pipeline has a missing trigger somewhere. See KNOWN_ISSUES.md §2.
//!
//! This test is the "B path" investigation target — once the puller chain is
//! fixed, remove `#[ignore]`.
//!
//! Originally intended verification surface:
//!
//! 1. TLS handshake + Hello exchange    (already covered by `e2e_handshake.rs`)
//! 2. **ClusterConfig exchange**         ← T2.5 (works ✅)
//! 3. **Index sync**                     ← T2.5 (works ✅)
//! 4. **Block transfer**                 ← T2.5 (BROKEN ❌)
//! 5. File materialization on receiver  ← T2.5 (BROKEN ❌)

use std::time::Duration;
use syncthing_core::types::Folder;
use syncthing_sync::SyncManager;
use syncthing_test_utils::harness::TestNode;

#[tokio::test]
#[ignore = "T2.5 diagnostic: exposes puller/index_handler chain bug; see KNOWN_ISSUES.md"]
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
    let file_b = folder_b_path.join("hello.txt");
    let start = std::time::Instant::now();
    let mut found = false;
    while start.elapsed() < Duration::from_secs(45) {
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
        "T2.5: file did not sync to node B within 45s - BEP bridge not driving full pipeline?"
    );
}
