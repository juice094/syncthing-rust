use super::*;

#[test]
fn test_message_header_bep_roundtrip() {
    let header = MessageHeader {
        message_type: MessageType::Ping,
        message_id: 0,
        compressed: false,
    };

    let bep = header.to_bep_header();
    let decoded = MessageHeader::from_bep_header(&bep).unwrap();

    assert_eq!(decoded.message_type, MessageType::Ping);
    assert!(!decoded.compressed);
}

#[test]
fn test_message_header_compression() {
    let header = MessageHeader {
        message_type: MessageType::Index,
        message_id: 0,
        compressed: true,
    };

    let bep = header.to_bep_header();
    assert_eq!(
        bep.compression,
        bep_protocol::messages::MessageCompression::Lz4 as i32
    );
    let decoded = MessageHeader::from_bep_header(&bep).unwrap();
    assert!(decoded.compressed);
}

#[tokio::test]
async fn test_split_boxed_pipe() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (pipe_a, pipe_b) = syncthing_test_utils::memory_pipe_pair(1024);
    let (_read_half, mut write_half) =
        tokio::io::split(Box::new(pipe_a) as syncthing_core::traits::BoxedPipe);
    let (mut read_half_b, _write_half_b) =
        tokio::io::split(Box::new(pipe_b) as syncthing_core::traits::BoxedPipe);

    write_half.write_all(b"hello").await.unwrap();
    write_half.flush().await.unwrap();
    drop(write_half);

    let mut buf = [0u8; 5];
    read_half_b.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello");
}

#[tokio::test]
async fn test_bep_connection_over_memory_pipe() {
    let (pipe_a, pipe_b) = syncthing_test_utils::memory_pipe_pair(4096);
    let (tx_a, _rx_a) = mpsc::unbounded_channel();
    let (tx_b, _rx_b) = mpsc::unbounded_channel();

    let conn_a = BepConnection::new(Box::new(pipe_a), ConnectionType::Outgoing, tx_a)
        .await
        .unwrap();

    let conn_b = BepConnection::new(Box::new(pipe_b), ConnectionType::Incoming, tx_b)
        .await
        .unwrap();

    // Send a Ping from A
    conn_a.send_ping().await.unwrap();

    // B should receive it
    let (msg_type, payload) = conn_b.recv_message().await.unwrap();
    assert_eq!(msg_type, MessageType::Ping);
    assert!(payload.is_empty());
}
