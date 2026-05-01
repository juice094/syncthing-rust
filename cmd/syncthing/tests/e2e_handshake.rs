//! E2E handshake test: two nodes discover each other and establish BEP connection.

use std::time::Duration;
use syncthing_core::types::Folder;
use syncthing_test_utils::harness::TestNode;

#[tokio::test]
async fn test_two_node_empty_folder_handshake() {
    let node_a = TestNode::new("a").await.expect("create node a");
    let node_b = TestNode::new("b").await.expect("create node b");

    // Create shared folders
    let folder_a_path = node_a.config_dir.join("sync");
    let folder_b_path = node_b.config_dir.join("sync");
    let mut folder_a = Folder::new("default", folder_a_path.to_str().unwrap());
    folder_a.devices.push(node_b.device_id);
    node_a.add_folder(folder_a).await.expect("a add folder");

    let mut folder_b = Folder::new("default", folder_b_path.to_str().unwrap());
    folder_b.devices.push(node_a.device_id);
    node_b.add_folder(folder_b).await.expect("b add folder");

    // Configure peers and initiate connections
    node_a.connect_to(&node_b).await.expect("a connect to b");
    node_b.connect_to(&node_a).await.expect("b connect to a");

    // Wait for connections to establish
    node_a.wait_for_connection(node_b.device_id, Duration::from_secs(15))
        .await
        .expect("a wait for b");
    node_b.wait_for_connection(node_a.device_id, Duration::from_secs(15))
        .await
        .expect("b wait for a");

    // Cleanup
    node_a.shutdown().await;
    node_b.shutdown().await;
}
