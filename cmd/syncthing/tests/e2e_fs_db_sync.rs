//! End-to-end test: encrypt a vault, sync it through syncthing-rust, decrypt on the other side.
//!
//! This test lives inside the syncthing-rust workspace so it can reuse TestNode and already
//! resolved dependencies. The encryption itself is performed by the external
//! `obsidian-vault-crypto-adapter` crate; syncthing-rust only sees and syncs ciphertext.

use obsidian_vault_crypto_adapter::CryptoAdapter;
use std::path::Path;
use std::time::Duration;
use syncthing_core::types::Folder;
use syncthing_sync::SyncManager;
use syncthing_test_utils::harness::TestNode;

#[tokio::test]
#[serial_test::serial]
async fn test_encrypted_vault_syncs_over_syncthing_fs_db() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init();

    // Password and salt shared between both sides.
    let password = "e2e-vault-password";
    let salt = obsidian_vault_crypto_adapter::generate_salt();

    let config_a =
        std::env::temp_dir().join(format!("crypto-sync-a-fs-{:x}", rand::random::<u64>()));
    let config_b =
        std::env::temp_dir().join(format!("crypto-sync-b-fs-{:x}", rand::random::<u64>()));
    let node_a =
        TestNode::new_with_dir_persistent("crypto-sync-a", config_a.clone(), config_a.join("db"))
            .await
            .expect("create node a");
    let node_b =
        TestNode::new_with_dir_persistent("crypto-sync-b", config_b.clone(), config_b.join("db"))
            .await
            .expect("create node b");

    let folder_a_path = node_a.config_dir.join("sync");
    let folder_b_path = node_b.config_dir.join("sync");

    // Configure folders on both nodes; syncthing-rust sees only the encrypted folder.
    let mut folder_a = Folder::new("crypto-e2e", folder_a_path.to_str().unwrap());
    folder_a.devices.push(node_b.device_id);
    node_a.add_folder(folder_a).await.expect("a add folder");

    let mut folder_b = Folder::new("crypto-e2e", folder_b_path.to_str().unwrap());
    folder_b.devices.push(node_a.device_id);
    node_b.add_folder(folder_b).await.expect("b add folder");

    // Build a plaintext vault on node A and encrypt it into the syncthing-monitored folder.
    let plaintext_vault_a = node_a.config_dir.join("vault_plain");
    std::fs::create_dir(&plaintext_vault_a).unwrap();
    std::fs::create_dir(plaintext_vault_a.join("Projects")).unwrap();
    std::fs::write(plaintext_vault_a.join("Daily.md"), "# Daily Note from A").unwrap();
    std::fs::write(
        plaintext_vault_a.join("Projects").join("Idea.md"),
        "# Project Idea from A",
    )
    .unwrap();

    let adapter_a = CryptoAdapter::from_password(password, &salt).unwrap();
    adapter_a
        .encrypt_dir(&plaintext_vault_a, &folder_a_path)
        .expect("encrypt vault a");

    // Scan folder A so the encrypted files enter the BEP index.
    node_a
        .sync_service
        .scan_folder("crypto-e2e")
        .await
        .expect("a scan folder");

    // Single-direction connect (Android-style): only node_b dials node_a via reconnect.
    node_b
        .reconnect_to(&node_a)
        .await
        .expect("b reconnect to a");

    node_a
        .wait_for_connection(node_b.device_id, Duration::from_secs(15))
        .await
        .expect("a wait for b");
    node_b
        .wait_for_connection(node_a.device_id, Duration::from_secs(15))
        .await
        .expect("b wait for a");

    // Wait for at least one encrypted file to land on node B.
    let start = std::time::Instant::now();
    let mut found = false;
    while start.elapsed() < Duration::from_secs(90) {
        let entries: Vec<_> = std::fs::read_dir(&folder_b_path)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();
        if !entries.is_empty() {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    assert!(found, "encrypted files did not sync to node B");

    // Decrypt node B's sync folder while its temp directory is still alive.
    let adapter_b = CryptoAdapter::from_password(password, &salt).unwrap();
    let plaintext_vault_b = node_b.config_dir.join("vault_restored");
    adapter_b
        .decrypt_dir(&folder_b_path, &plaintext_vault_b)
        .expect("decrypt vault b");

    // Compare original and decrypted plaintext.
    assert_files_equal(
        &plaintext_vault_a.join("Daily.md"),
        &plaintext_vault_b.join("Daily.md"),
    );
    assert_files_equal(
        &plaintext_vault_a.join("Projects").join("Idea.md"),
        &plaintext_vault_b.join("Projects").join("Idea.md"),
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
}

fn assert_files_equal(a: &Path, b: &Path) {
    let content_a = std::fs::read_to_string(a).unwrap();
    let content_b = std::fs::read_to_string(b).unwrap();
    assert_eq!(
        content_a,
        content_b,
        "file mismatch: {} vs {}",
        a.display(),
        b.display()
    );
}
