//! BEP Protocol Handshake
//!
//! 实现BEP协议的Hello消息交换
//! 参考: syncthing/lib/protocol/bep_hello.go

// bytes模块在此文件中不需要直接导入
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info};

use crate::messages::Hello;
use crate::{Result, SyncthingError};

/// Hello Magic Number (4 bytes, big-endian)
pub const HELLO_MAGIC: u32 = 0x2EA7D90B;

/// 最大Hello消息大小 (1KB)
pub const MAX_HELLO_SIZE: usize = 1024;

/// 旧版本协议Magic Number（用于错误检测）
const OLD_MAGIC_1: u32 = 0x00010000;
const OLD_MAGIC_2: u32 = 0x00010001;

/// 发送Hello消息
///
/// 消息格式:
/// ```text
/// [4 bytes]  Magic: 0x2EA7D90B
/// [2 bytes]  Length (big-endian)
/// [N bytes]  Protobuf encoded Hello message
/// ```
pub async fn send_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &Hello,
) -> Result<()> {
    // 编码Hello消息
    let msg_bytes = hello.encode_to_vec();

    if msg_bytes.len() > MAX_HELLO_SIZE {
        return Err(SyncthingError::protocol(format!(
            "Hello too large: {} > {}",
            msg_bytes.len(),
            MAX_HELLO_SIZE
        )));
    }

    debug!(
        "Sending Hello: device={}, client={}/{}",
        hello.device_name, hello.client_name, hello.client_version
    );

    // 写入Magic (4 bytes, big-endian)
    writer.write_u32(HELLO_MAGIC).await?;

    // 写入长度 (2 bytes, big-endian)
    writer.write_u16(msg_bytes.len() as u16).await?;

    // 写入消息
    writer.write_all(&msg_bytes).await?;

    // 刷新
    writer.flush().await?;

    info!(
        "Hello sent: device={} client={}/{} num_connections={}",
        hello.device_name, hello.client_name, hello.client_version, hello.num_connections
    );

    Ok(())
}

/// 接收Hello消息
///
/// 读取并解码对端发送的Hello消息
pub async fn recv_hello<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Hello> {
    // 读取Magic (4 bytes, big-endian)
    let magic = reader.read_u32().await?;

    if magic != HELLO_MAGIC {
        if magic == OLD_MAGIC_1 || magic == OLD_MAGIC_2 {
            return Err(SyncthingError::protocol(
                "the remote device speaks an older version of the protocol".to_string()
            ));
        }
        return Err(SyncthingError::protocol(format!(
            "unexpected magic: expected 0x{:08X}, got 0x{:08X}",
            HELLO_MAGIC, magic
        )));
    }

    // 读取长度 (2 bytes, big-endian)
    let len = reader.read_u16().await? as usize;

    if len > MAX_HELLO_SIZE {
        return Err(SyncthingError::protocol(format!(
            "Hello too large: {} > {}",
            len, MAX_HELLO_SIZE
        )));
    }

    // 读取消息体
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;

    // 解码Hello消息
    let hello = Hello::decode(&buf[..])?;

    info!(
        "Hello received: device={} client={}/{} num_connections={} timestamp={}",
        hello.device_name, hello.client_name, hello.client_version,
        hello.num_connections, hello.timestamp
    );

    Ok(hello)
}

/// 交换Hello消息
///
/// 发送我们的Hello并接收对方的Hello
pub async fn exchange_hello<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    our_hello: &Hello,
) -> Result<Hello> {
    // 发送我们的Hello
    send_hello(stream, our_hello).await?;

    // 接收对方的Hello
    let their_hello = recv_hello(stream).await?;

    info!(
        "Hello exchange complete: remote_device={}, client={}/{}",
        their_hello.device_name, their_hello.client_name, their_hello.client_version
    );

    Ok(their_hello)
}

/// 作为服务端交换Hello消息（先接收后发送）
///
/// 对于传入连接，我们需要先接收客户端的Hello，然后发送我们的响应
pub async fn exchange_hello_server<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    our_hello: &Hello,
) -> Result<Hello> {
    // 先接收客户端的Hello
    let their_hello = recv_hello(stream).await?;

    // 然后发送我们的Hello
    send_hello(stream, our_hello).await?;

    info!(
        "Hello exchange complete (server): remote_device={}, client={}/{}",
        their_hello.device_name, their_hello.client_name, their_hello.client_version
    );

    Ok(their_hello)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[tokio::test]
    async fn test_hello_roundtrip() {
        let hello = Hello {
            device_name: "test-device".to_string(),
            client_name: "syncthing-rust".to_string(),
            client_version: "0.1.0".to_string(),
            num_connections: 1,
            timestamp: 1234567890,
        };

        let mut buf = Vec::new();
        send_hello(&mut buf, &hello).await.unwrap();

        // 验证Magic
        assert_eq!(&buf[0..4], &[0x2E, 0xA7, 0xD9, 0x0B]);

        // 验证长度
        let len = u16::from_be_bytes([buf[4], buf[5]]) as usize;
        assert!(len > 0);

        let received = recv_hello(&mut &buf[..]).await.unwrap();
        assert_eq!(received.device_name, hello.device_name);
        assert_eq!(received.client_name, hello.client_name);
        assert_eq!(received.client_version, hello.client_version);
        assert_eq!(received.num_connections, hello.num_connections);
        assert_eq!(received.timestamp, hello.timestamp);
    }

    #[tokio::test]
    async fn test_exchange_hello() {
        let our_hello = Hello {
            device_name: "local-device".to_string(),
            client_name: "syncthing-rust".to_string(),
            client_version: "0.1.0".to_string(),
            num_connections: 1,
            timestamp: 1234567890,
        };

        // 模拟对端响应
        let their_hello = Hello {
            device_name: "remote-device".to_string(),
            client_name: "syncthing-go".to_string(),
            client_version: "1.27.0".to_string(),
            num_connections: 1,
            timestamp: 1234567891,
        };

        // 创建双向管道
        let (mut client, server) = tokio::io::duplex(1024);

        // 在单独任务中运行服务端
        let server_handle = tokio::spawn(async move {
            let mut server_clone = server;
            // 客户端先发Hello，所以服务端先接收
            let received = recv_hello(&mut server_clone).await.unwrap();
            // 然后发送响应
            send_hello(&mut server_clone, &their_hello).await.unwrap();
            received
        });

        // 客户端执行交换
        let result = exchange_hello(&mut client, &our_hello).await.unwrap();

        // 验证服务端收到的Hello
        let server_received = server_handle.await.unwrap();
        assert_eq!(server_received.device_name, "local-device");

        // 验证客户端收到的Hello
        assert_eq!(result.device_name, "remote-device");
        assert_eq!(result.client_name, "syncthing-go");
        assert_eq!(result.client_version, "1.27.0");
    }

    #[tokio::test]
    async fn test_invalid_magic() {
        let buf = vec![0x00, 0x00, 0x00, 0x00]; // Invalid magic
        let result = recv_hello(&mut &buf[..]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_old_magic_error() {
        // 旧版本协议Magic Number应该返回特定错误
        let mut buf = Vec::new();
        buf.extend_from_slice(&0x00010000u32.to_be_bytes());
        
        let result = recv_hello(&mut &buf[..]).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("older version"), "Error should mention older version: {}", err_msg);
    }

    #[tokio::test]
    async fn test_hello_too_large() {
        // 尝试发送超过最大大小的Hello消息
        let hello = Hello {
            device_name: "x".repeat(MAX_HELLO_SIZE + 1),
            client_name: String::new(),
            client_version: String::new(),
            num_connections: 0,
            timestamp: 0,
        };

        let mut buf = Vec::new();
        let result = send_hello(&mut buf, &hello).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_magic_constants() {
        assert_eq!(HELLO_MAGIC, 0x2EA7D90B);
        assert_eq!(MAX_HELLO_SIZE, 1024);
    }
}
