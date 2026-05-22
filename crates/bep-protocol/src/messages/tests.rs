use super::*;

#[test]
fn test_hello_encode_decode() {
    let hello = Hello {
        device_name: "test-device".to_string(),
        client_name: "syncthing-rust".to_string(),
        client_version: "0.1.0".to_string(),
        num_connections: 1,
        timestamp: 1234567890,
    };

    let encoded = hello.encode_to_vec();
    let decoded = Hello::decode(&encoded).unwrap();

    assert_eq!(decoded.device_name, hello.device_name);
    assert_eq!(decoded.client_name, hello.client_name);
    assert_eq!(decoded.client_version, hello.client_version);
    assert_eq!(decoded.num_connections, hello.num_connections);
    assert_eq!(decoded.timestamp, hello.timestamp);
}

#[test]
fn test_hello_default() {
    let hello = Hello::default();
    assert_eq!(hello.client_name, "syncthing-rust");
    assert_eq!(hello.client_version, "0.1.0");
    assert_eq!(hello.num_connections, 1);
}

#[test]
fn test_hello_new() {
    let hello = Hello::new("my-device", "test-client", "1.0.0");
    assert_eq!(hello.device_name, "my-device");
    assert_eq!(hello.client_name, "test-client");
    assert_eq!(hello.client_version, "1.0.0");
    assert_eq!(hello.num_connections, 1);
    assert!(hello.timestamp > 0);
}

#[test]
fn test_empty_hello() {
    let hello = Hello {
        device_name: String::new(),
        client_name: String::new(),
        client_version: String::new(),
        num_connections: 0,
        timestamp: 0,
    };

    let encoded = hello.encode_to_vec();
    assert!(encoded.is_empty());
}

#[test]
fn test_varint_roundtrip() {
    let test_values = [
        0u64,
        1,
        127,
        128,
        255,
        256,
        16383,
        16384,
        65535,
        65536,
        u32::MAX as u64,
    ];

    for &value in &test_values {
        let mut buf = BytesMut::new();
        put_varint(&mut buf, value);
        let (decoded, bytes_read) = read_varint(&buf).unwrap();
        assert_eq!(
            decoded, value,
            "varint {} encoded to {:?}, decoded to {}",
            value, buf, decoded
        );
        assert_eq!(bytes_read, buf.len());
    }
}

#[test]
fn test_request_roundtrip() {
    let req = Request {
        id: 42,
        folder: "default".to_string(),
        name: "test.txt".to_string(),
        offset: 1024,
        size: 256,
        hash: vec![0xab, 0xcd],
        from_temporary: false,
        block_no: 0,
    };
    let encoded = encode_message(&req).unwrap();
    let decoded: Request = decode_message(&encoded).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn test_response_roundtrip() {
    let resp = Response {
        id: 7,
        data: vec![1, 2, 3, 4],
        code: ErrorCode::NoError as i32,
    };
    let encoded = encode_message(&resp).unwrap();
    let decoded: Response = decode_message(&encoded).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn test_index_roundtrip() {
    let idx = Index {
        folder: "default".to_string(),
        files: vec![WireFileInfo {
            name: "foo".to_string(),
            r#type: FileInfoType::File as i32,
            size: 100,
            permissions: 0o644,
            modified_s: 12345,
            deleted: false,
            invalid: false,
            no_permissions: false,
            version: Some(WireVector {
                counters: vec![WireCounter { id: 1, value: 2 }],
            }),
            sequence: 1,
            modified_ns: 0,
            modified_by: 0,
            block_size: 0,
            platform: None,
            blocks: vec![WireBlockInfo {
                offset: 0,
                size: 10,
                hash: vec![0xde, 0xad],
            }],
            symlink_target: Vec::new(),
            blocks_hash: Vec::new(),
            encrypted: Vec::new(),
            previous_blocks_hash: Vec::new(),
        }],
        last_sequence: 0,
    };
    let encoded = encode_message(&idx).unwrap();
    let decoded: Index = decode_message(&encoded).unwrap();
    assert_eq!(idx, decoded);
}

#[test]
fn test_file_info_conversion() {
    let original = syncthing_core::types::FileInfo {
        name: "photo.jpg".to_string(),
        file_type: syncthing_core::types::FileType::File,
        size: 2048,
        permissions: 0o644,
        modified_s: 1600000000,
        modified_ns: 0,
        version: syncthing_core::types::Vector::new().with_counter(1, 5),
        sequence: 10,
        block_size: 128,
        blocks: vec![syncthing_core::types::BlockInfo {
            size: 128,
            hash: vec![0xca, 0xfe],
            offset: 0,
        }],
        symlink_target: None,
        deleted: Some(false),
        modified_by: None,
        blocks_hash: None,
        no_permissions: None,
    };

    let wire: WireFileInfo = original.clone().into();
    let back: syncthing_core::types::FileInfo = wire.into();

    assert_eq!(back.name, original.name);
    assert_eq!(back.size, original.size);
    assert_eq!(back.modified_s, original.modified_s);
    assert_eq!(back.version, original.version);
    assert_eq!(back.blocks.len(), original.blocks.len());
    assert_eq!(back.blocks[0].hash, original.blocks[0].hash);
    assert_eq!(back.blocks[0].size, original.blocks[0].size);
    assert_eq!(back.deleted, original.deleted);
    assert_eq!(back.sequence, original.sequence);
}

#[test]
fn test_cluster_config_roundtrip() {
    let cc = ClusterConfig {
        folders: vec![WireFolder {
            id: "default".to_string(),
            label: "Default Folder".to_string(),
            r#type: FolderType::SendReceive as i32,
            stop_reason: FolderStopReason::Running as i32,
            devices: vec![WireDevice {
                id: vec![0xab; 32],
                name: "test-device".to_string(),
                addresses: vec!["dynamic".to_string()],
                compression: Compression::Metadata as i32,
                cert_name: String::new(),
                max_sequence: 0,
                introducer: false,
                index_id: 0,
                skip_introduction_removals: false,
                encryption_password_token: Vec::new(),
            }],
        }],
        secondary: false,
    };
    let encoded = encode_message(&cc).unwrap();
    let decoded: ClusterConfig = decode_message(&encoded).unwrap();
    assert_eq!(cc, decoded);
}
