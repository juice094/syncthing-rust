//! Acceptance Tests for syncthing-net and end-to-end sync
//!
//! These tests verify that the network layer implementation meets
//! production requirements and that end-to-end sync works correctly.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use syncthing_core::{
    traits::{BepMessage, Discovery, SyncModel, Transport},
    types::{Compression, Config, DeviceConfig, DeviceId, FolderConfig, FolderId, GuiConfig, Options},
};
use syncthing_net::TcpTransport;
use syncthing_sync::SyncService;
use syncthing_api::config::MemoryConfigStore;

/// Test Configuration
const TEST_TIMEOUT: Duration = Duration::from_secs(30);
const LOCAL_ADDR: &str = "127.0.0.1:0";

/// NET-001 Acceptance: Transport can create endpoint and listen
#[tokio::test]
async fn net_001_transport_creation() {
    let result = timeout(TEST_TIMEOUT, async { TcpTransport::new() }).await;
    assert!(result.is_ok(), "Transport creation timed out");
    assert!(result.unwrap().is_ok(), "Transport creation failed");
}

/// NET-001 Acceptance: Transport can listen for connections
#[tokio::test]
async fn net_001_transport_listen() {
    let transport = TcpTransport::new().unwrap();
    
    let result: Result<Result<Box<dyn syncthing_core::traits::ConnectionListener>, _>, _> = timeout(
        TEST_TIMEOUT,
        transport.listen("127.0.0.1:22000")
    ).await;
    
    assert!(result.is_ok(), "Listen timed out");
    assert!(result.unwrap().is_ok(), "Listen failed");
}

/// NET-003 Acceptance: Discovery can announce and lookup
#[tokio::test]
async fn net_003_discovery_roundtrip() {
    let discovery = syncthing_net::IrohDiscovery::new();
    let device = test_device();
    let addresses = vec!["192.168.1.1:22000".to_string()];
    
    // Announce
    let announce_result = discovery.announce(&device, addresses.clone()).await;
    assert!(announce_result.is_ok(), "Announce failed");
    
    // Lookup (should find in local cache)
    let lookup_result = discovery.lookup(&device).await;
    assert!(lookup_result.is_ok(), "Lookup failed");
    assert_eq!(lookup_result.unwrap(), addresses, "Addresses don't match");
}

/// Integration: Two devices can discover each other
#[tokio::test]
async fn integration_device_discovery() {
    let discovery_a = syncthing_net::IrohDiscovery::new();
    let discovery_b = syncthing_net::IrohDiscovery::new();
    
    let device_a = test_device();
    let mut bytes = [0u8; 32];
    bytes[0] = 2;
    let _device_b = DeviceId::from_bytes(bytes);
    
    // Device A announces
    discovery_a.announce(&device_a, vec!["10.0.0.1:22000".to_string()]).await.unwrap();
    
    // Device B discovers A (in real impl, this uses DHT)
    // For now, we manually add
    discovery_b.add_device(device_a, vec!["10.0.0.1:22000".to_string()]).await;
    
    let found = discovery_b.lookup(&device_a).await.unwrap();
    assert!(!found.is_empty(), "Device not discovered");
}

/// Test block transfer end-to-end between two TcpTransports
#[tokio::test]
async fn test_block_transfer_end_to_end() {
    syncthing_net::init_crypto_provider();

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let cert_a = dir_a.path().join("cert.pem");
    let key_a = dir_a.path().join("key.pem");
    let cert_b = dir_b.path().join("cert.pem");
    let key_b = dir_b.path().join("key.pem");

    // Create two transports with certificates saved in temp dirs
    let transport_a = TcpTransport::new_with_cert_paths(&cert_a, &key_a).unwrap();
    let transport_b = TcpTransport::new_with_cert_paths(&cert_b, &key_b).unwrap();

    // B listens on ephemeral port
    let mut listener_b = transport_b.listen(LOCAL_ADDR).await.unwrap();
    let b_addr = listener_b.local_addr().unwrap();

    // B accepts connection in background and responds to Request messages
    let b_handle = tokio::spawn(async move {
        let mut conn = timeout(Duration::from_secs(10), listener_b.accept())
            .await
            .expect("B accept timed out")
            .expect("B accept failed")
            .expect("B accept returned None");

        let folder_id = FolderId::new("test-folder");
        let expected_data = b"hello from B".to_vec();
        loop {
            let msg = timeout(Duration::from_secs(10), conn.recv_message())
                .await
                .expect("B recv timed out")
                .expect("B recv failed")
                .expect("B connection closed");

            match msg {
                BepMessage::Request { id, folder, hash: _, offset: _, size: _ } => {
                    assert_eq!(folder, folder_id);
                    let response = BepMessage::Response {
                        id,
                        hash: syncthing_core::types::BlockHash::from_bytes([0u8; 32]),
                        data: expected_data.clone(),
                    };
                    conn.send_message(&response).await.expect("B send failed");
                    break;
                }
                _ => {
                    // Ignore other messages (e.g., ClusterConfig, Ping)
                }
            }
        }
        conn
    });

    // A connects to B
    let mut conn_a = timeout(
        Duration::from_secs(10),
        transport_a.connect(&format!("tcp://{}", b_addr), None),
    )
    .await
    .expect("A connect timed out")
    .expect("A connect failed");

    // A requests a block
    let folder_id = FolderId::new("test-folder");
    let hash = syncthing_core::types::BlockHash::from_bytes([0u8; 32]);
    let data = timeout(
        Duration::from_secs(10),
        conn_a.request_block(&folder_id, hash, 0, 16),
    )
    .await
    .expect("A request_block timed out")
    .expect("A request_block failed");

    assert_eq!(data, b"hello from B");

    // Wait for B responder to finish
    b_handle.await.expect("B responder task failed");
}

/// Test minimal local sync between two SyncServices
#[tokio::test]
async fn test_minimal_local_sync() {
    syncthing_net::init_crypto_provider();
    let _subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let data_dir_a = dir_a.path().join("data");
    let data_dir_b = dir_b.path().join("data");
    let sync_dir_a = dir_a.path().join("shared");
    let sync_dir_b = dir_b.path().join("shared");
    std::fs::create_dir_all(&sync_dir_a).unwrap();
    std::fs::create_dir_all(&sync_dir_b).unwrap();

    // Create a test file in A
    let test_file_a = sync_dir_a.join("test.txt");
    std::fs::write(&test_file_a, b"Hello, Syncthing!").unwrap();

    // Pre-generate certificates to get device IDs
    let cert_path_a = data_dir_a.join("cert.pem");
    let key_path_a = data_dir_a.join("key.pem");
    let transport_a = TcpTransport::new_with_cert_paths(&cert_path_a, &key_path_a).unwrap();
    let device_a = transport_a.device_id();

    let cert_path_b = data_dir_b.join("cert.pem");
    let key_path_b = data_dir_b.join("key.pem");
    let transport_b = TcpTransport::new_with_cert_paths(&cert_path_b, &key_path_b).unwrap();
    let device_b = transport_b.device_id();

    // Build config for A
    let folder_id = FolderId::new("test-folder");
    let mut folder_config_a = FolderConfig::new(folder_id.clone(), &sync_dir_a);
    folder_config_a.devices = vec![device_a, device_b];

    let config_a = Config {
        version: 37,
        folders: vec![folder_config_a],
        devices: vec![
            DeviceConfig {
                id: device_a,
                name: "Device A".to_string(),
                addresses: vec!["dynamic".to_string()],
                introducer: false,
                compression: Compression::Metadata,
            },
            DeviceConfig {
                id: device_b,
                name: "Device B".to_string(),
                addresses: vec!["dynamic".to_string()],
                introducer: false,
                compression: Compression::Metadata,
            },
        ],
        gui: GuiConfig {
            enabled: false,
            address: "127.0.0.1:8384".to_string(),
            api_key: None,
            use_tls: false,
        },
        options: Options {
            listen_addresses: vec![format!("tcp://{}", LOCAL_ADDR)],
            global_discovery: false,
            local_discovery: false,
            nat_traversal: false,
            relays_enabled: false,
        },
    };

    let config_store_a = Arc::new(MemoryConfigStore::new());
    let (mut sync_service_a, _) = SyncService::new(config_a, config_store_a, data_dir_a)
        .await
        .expect("Failed to create SyncService A");

    // Start A and get its actual listen address
    sync_service_a.start().await.expect("Failed to start SyncService A");
    let a_addr = sync_service_a
        .listen_addr()
        .await
        .expect("A should have a listen address");

    // Build config for B, pointing to A's actual address
    let mut folder_config_b = FolderConfig::new(folder_id.clone(), &sync_dir_b);
    folder_config_b.devices = vec![device_a, device_b];

    let config_b = Config {
        version: 37,
        folders: vec![folder_config_b],
        devices: vec![
            DeviceConfig {
                id: device_a,
                name: "Device A".to_string(),
                addresses: vec![format!("tcp://{}", a_addr)],
                introducer: false,
                compression: Compression::Metadata,
            },
            DeviceConfig {
                id: device_b,
                name: "Device B".to_string(),
                addresses: vec!["dynamic".to_string()],
                introducer: false,
                compression: Compression::Metadata,
            },
        ],
        gui: GuiConfig {
            enabled: false,
            address: "127.0.0.1:8384".to_string(),
            api_key: None,
            use_tls: false,
        },
        options: Options {
            listen_addresses: vec![format!("tcp://{}", LOCAL_ADDR)],
            global_discovery: false,
            local_discovery: false,
            nat_traversal: false,
            relays_enabled: false,
        },
    };

    let config_store_b = Arc::new(MemoryConfigStore::new());
    let (mut sync_service_b, _) = SyncService::new(config_b, config_store_b, data_dir_b)
        .await
        .expect("Failed to create SyncService B");

    // Start B
    sync_service_b.start().await.expect("Failed to start SyncService B");

    // Allow time for connection establishment and index exchange
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Trigger pull on B
    let pull_result = timeout(
        Duration::from_secs(15),
        sync_service_b.sync_engine().pull(&folder_id),
    )
    .await
    .expect("B pull timed out")
    .expect("B pull failed");

    assert_eq!(
        pull_result.files_processed, 1,
        "Expected 1 file processed, got {:?}", pull_result
    );

    // Verify the file was synced
    let test_file_b = sync_dir_b.join("test.txt");
    assert!(test_file_b.exists(), "test.txt should exist on B after sync");
    let content_a = std::fs::read(&test_file_a).unwrap();
    let content_b = std::fs::read(&test_file_b).unwrap();
    assert_eq!(content_b, content_a, "File content should match after sync");

    // Cleanup
    sync_service_a.stop().await;
    sync_service_b.stop().await;
}

// Test helpers
fn test_device() -> DeviceId {
    let mut bytes = [0u8; 32];
    bytes[0] = 1;
    DeviceId::from_bytes(bytes)
}
