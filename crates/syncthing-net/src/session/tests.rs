use std::sync::atomic::Ordering;

use super::*;
use crate::protocol::MessageType;
use syncthing_core::ConnectionType;

struct MockHandler {
    index_calls: tokio::sync::Mutex<Vec<(String, DeviceId)>>,
}

impl MockHandler {
    fn new() -> Self {
        Self {
            index_calls: tokio::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl BepSessionHandler for MockHandler {
    async fn generate_cluster_config(
        &self,
        _device_id: DeviceId,
    ) -> Result<bep_protocol::messages::ClusterConfig> {
        Ok(bep_protocol::messages::ClusterConfig {
            folders: vec![bep_protocol::messages::WireFolder {
                id: "test-folder".to_string(),
                label: String::new(),
                r#type: 0,
                stop_reason: 0,
                devices: vec![],
            }],
            secondary: false,
        })
    }

    async fn generate_index(
        &self,
        folder_id: &str,
        device_id: DeviceId,
    ) -> Result<syncthing_core::types::Index> {
        self.index_calls
            .lock()
            .await
            .push((folder_id.to_string(), device_id));
        Ok(syncthing_core::types::Index {
            folder: folder_id.to_string(),
            files: vec![],
        })
    }

    async fn on_index(
        &self,
        _device_id: DeviceId,
        _index: syncthing_core::types::Index,
    ) -> Result<()> {
        Ok(())
    }

    async fn on_index_update(
        &self,
        _device_id: DeviceId,
        _update: syncthing_core::types::IndexUpdate,
    ) -> Result<()> {
        Ok(())
    }

    async fn on_block_request(
        &self,
        _device_id: DeviceId,
        _req: bep_protocol::messages::Request,
    ) -> std::result::Result<Vec<u8>, bep_protocol::messages::ErrorCode> {
        Ok(vec![1, 2, 3])
    }
}

#[tokio::test]
async fn test_session_ping_pong() {
    let (pipe_a, pipe_b) = syncthing_test_utils::memory_pipe_pair(4096);
    let (tx_a, _rx_a) = tokio::sync::mpsc::channel(256);
    let (tx_b, _rx_b) = tokio::sync::mpsc::channel(256);

    let conn_a = BepConnection::new(Box::new(pipe_a), ConnectionType::Outgoing, tx_a)
        .await
        .unwrap();
    let conn_b = BepConnection::new(Box::new(pipe_b), ConnectionType::Incoming, tx_b)
        .await
        .unwrap();

    let device_id = DeviceId::default();
    let handler = Arc::new(MockHandler::new());
    let pending: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>> =
        Arc::new(DashMap::new());

    let session = BepSession::new(
        Arc::new(syncthing_core::DeviceIdentity::new(device_id)),
        Arc::clone(&conn_a),
        handler,
        pending,
    );
    let handle = tokio::spawn(session.run());

    // Wait for ClusterConfig from session side
    let (msg_type, _) = conn_b.recv_message().await.unwrap();
    assert_eq!(msg_type, MessageType::ClusterConfig);

    // Reply with ClusterConfig
    let reply_cc = bep_protocol::messages::ClusterConfig {
        folders: vec![bep_protocol::messages::WireFolder {
            id: "test-folder".to_string(),
            label: String::new(),
            r#type: 0,
            stop_reason: 0,
            devices: vec![],
        }],
        secondary: false,
    };
    let payload = bep_protocol::messages::encode_message(&reply_cc).unwrap();
    conn_b
        .send_message(MessageType::ClusterConfig, payload)
        .await
        .unwrap();

    // Wait for Index
    let (msg_type, _) = conn_b.recv_message().await.unwrap();
    assert_eq!(msg_type, MessageType::Index);

    // Send a Ping
    conn_b.send_ping().await.unwrap();

    // BEP Ping 是单向 keepalive：不应触发回复。
    // 会话自身心跳定时器的首次 tick 立即触发，窗口内可能恰好有 1 条 Ping；
    // 若是旧的"收 Ping 回 Ping"互答行为，窗口内会收到大量 Ping（风暴）。
    let mut ping_count = 0u32;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        match tokio::time::timeout(deadline - now, conn_b.recv_message()).await {
            Ok(Ok((MessageType::Ping, _))) => ping_count += 1,
            Ok(Ok(_)) => {}
            _ => break,
        }
    }
    assert!(
        ping_count <= 1,
        "expected at most 1 heartbeat Ping, got {} (ping storm?)",
        ping_count
    );

    // Clean shutdown
    conn_b.close().await.ok();
    handle.abort();
}

#[tokio::test]
async fn test_session_block_request_response() {
    let (pipe_a, pipe_b) = syncthing_test_utils::memory_pipe_pair(4096);
    let (tx_a, _rx_a) = tokio::sync::mpsc::channel(256);
    let (tx_b, _rx_b) = tokio::sync::mpsc::channel(256);

    let conn_a = BepConnection::new(Box::new(pipe_a), ConnectionType::Outgoing, tx_a)
        .await
        .unwrap();
    let conn_b = BepConnection::new(Box::new(pipe_b), ConnectionType::Incoming, tx_b)
        .await
        .unwrap();

    let device_id = DeviceId::default();
    let handler = Arc::new(MockHandler::new());
    let pending: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>> =
        Arc::new(DashMap::new());

    let session = BepSession::new(
        Arc::new(syncthing_core::DeviceIdentity::new(device_id)),
        Arc::clone(&conn_a),
        handler,
        Arc::clone(&pending),
    );
    let handle = tokio::spawn(session.run());

    // Handshake: ClusterConfig -> ClusterConfig -> Index
    let (msg_type, _) = conn_b.recv_message().await.unwrap();
    assert_eq!(msg_type, MessageType::ClusterConfig);

    let reply_cc = bep_protocol::messages::ClusterConfig {
        folders: vec![bep_protocol::messages::WireFolder {
            id: "test-folder".to_string(),
            label: String::new(),
            r#type: 0,
            stop_reason: 0,
            devices: vec![],
        }],
        secondary: false,
    };
    let payload = bep_protocol::messages::encode_message(&reply_cc).unwrap();
    conn_b
        .send_message(MessageType::ClusterConfig, payload)
        .await
        .unwrap();

    let (msg_type, _) = conn_b.recv_message().await.unwrap();
    assert_eq!(msg_type, MessageType::Index);

    // Send a Request from B side
    let req = bep_protocol::messages::Request {
        id: 42,
        folder: "test".to_string(),
        name: "file.txt".to_string(),
        offset: 0,
        size: 3,
        hash: vec![],
        from_temporary: false,
        block_no: 0,
    };
    let req_payload = bep_protocol::messages::encode_message(&req).unwrap();
    conn_b
        .send_message(MessageType::Request, req_payload)
        .await
        .unwrap();

    // Should receive Response with mock data [1, 2, 3].
    // Session 会周期性发送 Ping/Pong，因此需要跳过这些心跳消息。
    let (msg_type, resp_payload) = loop {
        let (msg_type, payload) = conn_b.recv_message().await.unwrap();
        if msg_type == MessageType::Response {
            break (msg_type, payload);
        }
    };
    assert_eq!(msg_type, MessageType::Response);
    let resp =
        bep_protocol::messages::decode_message::<bep_protocol::messages::Response>(&resp_payload)
            .unwrap();
    assert_eq!(resp.id, 42);
    assert_eq!(resp.data, vec![1, 2, 3]);
    assert_eq!(resp.code, bep_protocol::messages::ErrorCode::NoError as i32);

    conn_b.close().await.ok();
    handle.abort();
}

#[tokio::test]
async fn test_session_events_and_metrics() {
    let (pipe_a, pipe_b) = syncthing_test_utils::memory_pipe_pair(4096);
    let (tx_a, _rx_a) = tokio::sync::mpsc::channel(256);
    let (tx_b, _rx_b) = tokio::sync::mpsc::channel(256);

    let conn_a = BepConnection::new(Box::new(pipe_a), ConnectionType::Outgoing, tx_a)
        .await
        .unwrap();
    let conn_b = BepConnection::new(Box::new(pipe_b), ConnectionType::Incoming, tx_b)
        .await
        .unwrap();

    let device_id = DeviceId::default();
    let handler = Arc::new(MockHandler::new());
    let pending: Arc<DashMap<i32, tokio::sync::oneshot::Sender<bep_protocol::messages::Response>>> =
        Arc::new(DashMap::new());

    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<BepSessionEvent>(256);
    let session = BepSession::with_events(
        Arc::new(syncthing_core::DeviceIdentity::new(device_id)),
        Arc::clone(&conn_a),
        handler,
        Arc::clone(&pending),
        event_tx,
    );
    let metrics = session.metrics();
    let handle = tokio::spawn(session.run());

    // 1. Expect ClusterConfig from session side
    let (msg_type, _) = conn_b.recv_message().await.unwrap();
    assert_eq!(msg_type, MessageType::ClusterConfig);

    // 2. Reply with ClusterConfig
    let reply_cc = bep_protocol::messages::ClusterConfig {
        folders: vec![bep_protocol::messages::WireFolder {
            id: "test-folder".to_string(),
            label: String::new(),
            r#type: 0,
            stop_reason: 0,
            devices: vec![],
        }],
        secondary: false,
    };
    let payload = bep_protocol::messages::encode_message(&reply_cc).unwrap();
    conn_b
        .send_message(MessageType::ClusterConfig, payload)
        .await
        .unwrap();

    // Wait for ClusterConfigComplete event
    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        BepSessionEvent::ClusterConfigComplete { .. }
    ));

    // 3. Expect Index from session side
    let (msg_type, _) = conn_b.recv_message().await.unwrap();
    assert_eq!(msg_type, MessageType::Index);

    // Wait for IndexSent event
    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, BepSessionEvent::IndexSent { folder, .. } if folder == "test-folder"));

    // 4. Send an Index from B side -> should trigger IndexReceived
    let index = bep_protocol::messages::Index {
        folder: "test-folder".to_string(),
        files: vec![bep_protocol::messages::WireFileInfo {
            name: "hello.txt".to_string(),
            ..Default::default()
        }],
        last_sequence: 0,
    };
    let idx_payload = bep_protocol::messages::encode_message(&index).unwrap();
    conn_b
        .send_message(MessageType::Index, idx_payload)
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, BepSessionEvent::IndexReceived { folder, file_count: 1, .. } if folder == "test-folder")
    );

    // Consume the PeerSyncState event emitted right after IndexReceived
    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, BepSessionEvent::PeerSyncState { folder, .. } if folder == "test-folder")
    );

    // 5. Send a Request -> should trigger BlockRequested + Response
    let req = bep_protocol::messages::Request {
        id: 7,
        folder: "test-folder".to_string(),
        name: "hello.txt".to_string(),
        offset: 0,
        size: 3,
        hash: vec![],
        from_temporary: false,
        block_no: 0,
    };
    let req_payload = bep_protocol::messages::encode_message(&req).unwrap();
    conn_b
        .send_message(MessageType::Request, req_payload)
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, BepSessionEvent::BlockRequested { folder, name, size: 3, .. }
        if folder == "test-folder" && name == "hello.txt")
    );

    // Receive Response (skip stray Ping messages that may arrive from heartbeat)
    let (msg_type, _) = loop {
        let (msg_type, payload) = conn_b.recv_message().await.unwrap();
        if msg_type != MessageType::Ping {
            break (msg_type, payload);
        }
    };
    assert_eq!(msg_type, MessageType::Response);

    // 6. Verify metrics (small yield to let async counters settle)
    tokio::time::sleep(Duration::from_millis(50)).await;
    let recv = metrics.messages_recv.load(Ordering::Relaxed);
    let sent = metrics.messages_sent.load(Ordering::Relaxed);
    assert!(recv >= 2, "expected at least 2 messages_recv, got {}", recv); // Index + Request
    assert_eq!(metrics.blocks_requested.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.blocks_served.load(Ordering::Relaxed), 1);
    assert!(sent >= 2, "expected at least 2 messages_sent, got {}", sent); // Ping reply + Response

    conn_b.close().await.ok();
    handle.abort();
}
