//! Syncthing Relay Protocol v1 消息定义与 XDR 编解码
//!
//! 协议规范: <https://docs.syncthing.net/specs/relay-v1.html>
//!
//! 所有多字节整数采用大端序（XDR 标准）。

use bytes::{Buf, BufMut, Bytes, BytesMut};

/// Relay 协议魔数
pub const MAGIC: u32 = 0x9E79BC40;

/// 消息类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MessageType {
    /// Ping (Type = 0)
    Ping = 0,
    /// Pong (Type = 1)
    Pong = 1,
    /// JoinRelayRequest (Type = 2)
    JoinRelayRequest = 2,
    /// JoinSessionRequest (Type = 3)
    JoinSessionRequest = 3,
    /// Response (Type = 4)
    Response = 4,
    /// ConnectRequest (Type = 5)
    ConnectRequest = 5,
    /// SessionInvitation (Type = 6)
    SessionInvitation = 6,
    /// RelayFull (Type = 7)
    RelayFull = 7,
}

impl MessageType {
    /// 从 i32 解析消息类型
    pub const fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Ping),
            1 => Some(Self::Pong),
            2 => Some(Self::JoinRelayRequest),
            3 => Some(Self::JoinSessionRequest),
            4 => Some(Self::Response),
            5 => Some(Self::ConnectRequest),
            6 => Some(Self::SessionInvitation),
            7 => Some(Self::RelayFull),
            _ => None,
        }
    }
}

/// 消息头（12 字节）
#[derive(Debug, Clone, Copy)]
pub struct Header {
    /// 魔数（应为 MAGIC）
    pub magic: u32,
    /// 消息类型
    pub message_type: MessageType,
    /// 消息体长度（不含头）
    pub message_length: i32,
}

impl Header {
    /// 头部长度（字节）
    pub const SIZE: usize = 12;

    /// 编码到 buf
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u32(self.magic);
        buf.put_i32(self.message_type as i32);
        buf.put_i32(self.message_length);
    }

    /// 从 buf 解码
    pub fn decode(buf: &mut Bytes) -> Option<Self> {
        if buf.remaining() < Self::SIZE {
            return None;
        }
        let magic = buf.get_u32();
        let msg_type = buf.get_i32();
        let msg_len = buf.get_i32();
        MessageType::from_i32(msg_type).map(|mt| Self {
            magic,
            message_type: mt,
            message_length: msg_len,
        })
    }
}

/// Ping 消息 (Type = 0)
#[derive(Debug, Clone)]
pub struct Ping;

impl Ping {
    /// 编码为完整消息（含 header）
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(Header::SIZE);
        Header {
            magic: MAGIC,
            message_type: MessageType::Ping,
            message_length: 0,
        }
        .encode(&mut buf);
        buf.freeze()
    }
}

/// Pong 消息 (Type = 1)
#[derive(Debug, Clone)]
pub struct Pong;

impl Pong {
    /// 编码为完整消息（含 header）
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(Header::SIZE);
        Header {
            magic: MAGIC,
            message_type: MessageType::Pong,
            message_length: 0,
        }
        .encode(&mut buf);
        buf.freeze()
    }
}

/// JoinRelayRequest 消息 (Type = 2)
#[derive(Debug, Clone)]
pub struct JoinRelayRequest;

impl JoinRelayRequest {
    /// 编码为完整消息（含 header）
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(Header::SIZE);
        Header {
            magic: MAGIC,
            message_type: MessageType::JoinRelayRequest,
            message_length: 0,
        }
        .encode(&mut buf);
        buf.freeze()
    }
}

/// JoinSessionRequest 消息 (Type = 3)
#[derive(Debug, Clone)]
pub struct JoinSessionRequest {
    /// 会话密钥
    pub key: Vec<u8>,
}

impl JoinSessionRequest {
    /// 编码为完整消息（含 header）
    pub fn encode(&self) -> Bytes {
        let payload_len = 4 + padded_len(self.key.len());
        let mut buf = BytesMut::with_capacity(Header::SIZE + payload_len);
        Header {
            magic: MAGIC,
            message_type: MessageType::JoinSessionRequest,
            message_length: payload_len as i32,
        }
        .encode(&mut buf);
        encode_opaque(&mut buf, &self.key);
        buf.freeze()
    }

    /// 从消息体解码（不含 header）
    pub fn decode(body: &mut Bytes) -> Option<Self> {
        let key = decode_opaque(body)?;
        Some(Self { key })
    }
}

/// Response 消息 (Type = 4)
#[derive(Debug, Clone)]
pub struct Response {
    /// 状态码（0=success, 1=not found, 2=already connected, 99=internal error, 100=unexpected message）
    pub code: i32,
    /// 状态描述
    pub message: String,
}

impl Response {
    /// 编码为完整消息（含 header）
    pub fn encode(&self) -> Bytes {
        let msg_bytes = self.message.as_bytes();
        let payload_len = 4 + 4 + padded_len(msg_bytes.len());
        let mut buf = BytesMut::with_capacity(Header::SIZE + payload_len);
        Header {
            magic: MAGIC,
            message_type: MessageType::Response,
            message_length: payload_len as i32,
        }
        .encode(&mut buf);
        buf.put_i32(self.code);
        encode_string(&mut buf, &self.message);
        buf.freeze()
    }

    /// 从消息体解码（不含 header）
    pub fn decode(body: &mut Bytes) -> Option<Self> {
        if body.remaining() < 4 {
            return None;
        }
        let code = body.get_i32();
        let message = decode_string(body)?;
        Some(Self { code, message })
    }

    /// 构造成功响应
    pub fn success() -> Self {
        Self {
            code: 0,
            message: "success".to_string(),
        }
    }

    /// 构造 not found 响应
    pub fn not_found() -> Self {
        Self {
            code: 1,
            message: "not found".to_string(),
        }
    }

    /// 构造 already connected 响应
    pub fn already_connected() -> Self {
        Self {
            code: 2,
            message: "already connected".to_string(),
        }
    }
}

/// ConnectRequest 消息 (Type = 5)
#[derive(Debug, Clone)]
pub struct ConnectRequest {
    /// 目标设备 ID（32 字节 raw bytes）
    pub id: Vec<u8>,
}

impl ConnectRequest {
    /// 编码为完整消息（含 header）
    pub fn encode(&self) -> Bytes {
        let payload_len = 4 + padded_len(self.id.len());
        let mut buf = BytesMut::with_capacity(Header::SIZE + payload_len);
        Header {
            magic: MAGIC,
            message_type: MessageType::ConnectRequest,
            message_length: payload_len as i32,
        }
        .encode(&mut buf);
        encode_opaque(&mut buf, &self.id);
        buf.freeze()
    }

    /// 从消息体解码（不含 header）
    pub fn decode(body: &mut Bytes) -> Option<Self> {
        let id = decode_opaque(body)?;
        Some(Self { id })
    }
}

/// SessionInvitation 消息 (Type = 6)
#[derive(Debug, Clone)]
pub struct SessionInvitation {
    /// 来源设备 ID
    pub from: Vec<u8>,
    /// 会话密钥
    pub key: Vec<u8>,
    /// 期望连接的地址（空表示使用 protocol mode 连接的同一 relay IP）
    pub address: Vec<u8>,
    /// 期望连接的端口
    pub port: u32,
    /// 本端应假设为 server socket（另一端为 client socket）
    pub server_socket: bool,
}

impl SessionInvitation {
    /// 编码为完整消息（含 header）
    pub fn encode(&self) -> Bytes {
        let payload_len = 4
            + padded_len(self.from.len())
            + 4
            + padded_len(self.key.len())
            + 4
            + padded_len(self.address.len())
            + 4
            + 4;
        let mut buf = BytesMut::with_capacity(Header::SIZE + payload_len);
        Header {
            magic: MAGIC,
            message_type: MessageType::SessionInvitation,
            message_length: payload_len as i32,
        }
        .encode(&mut buf);
        encode_opaque(&mut buf, &self.from);
        encode_opaque(&mut buf, &self.key);
        encode_opaque(&mut buf, &self.address);
        buf.put_u32(self.port);
        buf.put_u32(self.server_socket as u32);
        buf.freeze()
    }

    /// 从消息体解码（不含 header）
    pub fn decode(body: &mut Bytes) -> Option<Self> {
        let from = decode_opaque(body)?;
        let key = decode_opaque(body)?;
        let address = decode_opaque(body)?;
        if body.remaining() < 8 {
            return None;
        }
        let port = body.get_u32();
        let server_socket = body.get_u32() != 0;
        Some(Self {
            from,
            key,
            address,
            port,
            server_socket,
        })
    }
}

/// RelayFull 消息 (Type = 7)
#[derive(Debug, Clone)]
pub struct RelayFull;

impl RelayFull {
    /// 编码为完整消息（含 header）
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(Header::SIZE);
        Header {
            magic: MAGIC,
            message_type: MessageType::RelayFull,
            message_length: 0,
        }
        .encode(&mut buf);
        buf.freeze()
    }
}

/// 统一消息枚举
#[derive(Debug, Clone)]
pub enum Message {
    /// Ping
    Ping(Ping),
    /// Pong
    Pong(Pong),
    /// JoinRelayRequest
    JoinRelayRequest(JoinRelayRequest),
    /// JoinSessionRequest
    JoinSessionRequest(JoinSessionRequest),
    /// Response
    Response(Response),
    /// ConnectRequest
    ConnectRequest(ConnectRequest),
    /// SessionInvitation
    SessionInvitation(SessionInvitation),
    /// RelayFull
    RelayFull(RelayFull),
}

impl Message {
    /// 编码为字节（含 header）
    pub fn encode(&self) -> Bytes {
        match self {
            Self::Ping(m) => m.encode(),
            Self::Pong(m) => m.encode(),
            Self::JoinRelayRequest(m) => m.encode(),
            Self::JoinSessionRequest(m) => m.encode(),
            Self::Response(m) => m.encode(),
            Self::ConnectRequest(m) => m.encode(),
            Self::SessionInvitation(m) => m.encode(),
            Self::RelayFull(m) => m.encode(),
        }
    }
}

// --- XDR 辅助函数 ---

/// 计算填充后的长度（4 字节对齐）
const fn padded_len(len: usize) -> usize {
    let rem = len % 4;
    if rem == 0 {
        len
    } else {
        len + (4 - rem)
    }
}

/// 编码变长 opaque 数据（XDR: [length: u32] [data] [padding]）
fn encode_opaque(buf: &mut BytesMut, data: &[u8]) {
    buf.put_u32(data.len() as u32);
    buf.extend_from_slice(data);
    let pad = (4 - (data.len() % 4)) % 4;
    if pad > 0 {
        buf.extend_from_slice(&[0; 4][..pad]);
    }
}

/// 解码变长 opaque 数据
fn decode_opaque(buf: &mut Bytes) -> Option<Vec<u8>> {
    if buf.remaining() < 4 {
        return None;
    }
    let len = buf.get_u32() as usize;
    let padded = padded_len(len);
    if buf.remaining() < padded {
        return None;
    }
    let data = buf.copy_to_bytes(len).to_vec();
    if padded > len {
        buf.advance(padded - len);
    }
    Some(data)
}

/// 编码字符串（XDR string = opaque bytes of UTF-8）
fn encode_string(buf: &mut BytesMut, s: &str) {
    encode_opaque(buf, s.as_bytes());
}

/// 解码字符串
fn decode_string(buf: &mut Bytes) -> Option<String> {
    let bytes = decode_opaque(buf)?;
    String::from_utf8(bytes).ok()
}

// --- 异步 I/O 辅助函数 ---

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 从异步 reader 读取一条完整消息
pub async fn read_message<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> crate::relay::types::Result<Message> {
    use crate::relay::types::RelayError;

    let mut header_buf = [0u8; Header::SIZE];
    reader
        .read_exact(&mut header_buf)
        .await
        .map_err(|e| RelayError::Protocol(format!("read header: {}", e)))?;
    let mut header_bytes = Bytes::copy_from_slice(&header_buf);
    let header = Header::decode(&mut header_bytes)
        .ok_or_else(|| RelayError::Protocol("invalid header".to_string()))?;

    if header.magic != MAGIC {
        return Err(RelayError::Protocol(format!(
            "bad magic: {:08x}",
            header.magic
        )));
    }

    let body_len = header.message_length as usize;
    if body_len > 1024 * 1024 {
        // 1 MiB 上限，防止恶意消息
        return Err(RelayError::Protocol(format!(
            "message too large: {} bytes",
            body_len
        )));
    }

    let mut body_buf = vec![0u8; body_len];
    if body_len > 0 {
        reader
            .read_exact(&mut body_buf)
            .await
            .map_err(|e| RelayError::Protocol(format!("read body: {}", e)))?;
    }

    let mut body = Bytes::from(body_buf);
    let msg = match header.message_type {
        MessageType::Ping => Message::Ping(Ping),
        MessageType::Pong => Message::Pong(Pong),
        MessageType::JoinRelayRequest => Message::JoinRelayRequest(JoinRelayRequest),
        MessageType::JoinSessionRequest => Message::JoinSessionRequest(
            JoinSessionRequest::decode(&mut body)
                .ok_or_else(|| RelayError::Protocol("bad JoinSessionRequest".to_string()))?,
        ),
        MessageType::Response => Message::Response(
            Response::decode(&mut body)
                .ok_or_else(|| RelayError::Protocol("bad Response".to_string()))?,
        ),
        MessageType::ConnectRequest => Message::ConnectRequest(
            ConnectRequest::decode(&mut body)
                .ok_or_else(|| RelayError::Protocol("bad ConnectRequest".to_string()))?,
        ),
        MessageType::SessionInvitation => Message::SessionInvitation(
            SessionInvitation::decode(&mut body)
                .ok_or_else(|| RelayError::Protocol("bad SessionInvitation".to_string()))?,
        ),
        MessageType::RelayFull => Message::RelayFull(RelayFull),
    };
    Ok(msg)
}

/// 向异步 writer 写入一条完整消息
pub async fn write_message<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg: &Message,
) -> crate::relay::types::Result<()> {
    use crate::relay::types::RelayError;

    let bytes = msg.encode();
    writer
        .write_all(&bytes)
        .await
        .map_err(|e| RelayError::Protocol(format!("write: {}", e)))?;
    writer
        .flush()
        .await
        .map_err(|e| RelayError::Protocol(format!("flush: {}", e)))?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_encode_decode() {
        let mut buf = BytesMut::new();
        let header = Header {
            magic: MAGIC,
            message_type: MessageType::Ping,
            message_length: 0,
        };
        header.encode(&mut buf);
        assert_eq!(buf.len(), Header::SIZE);

        let mut bytes = buf.freeze();
        let decoded = Header::decode(&mut bytes).unwrap();
        assert_eq!(decoded.magic, MAGIC);
        assert_eq!(decoded.message_type, MessageType::Ping);
        assert_eq!(decoded.message_length, 0);
    }

    #[test]
    fn test_ping_pong_roundtrip() {
        let ping_bytes = Ping.encode();
        assert_eq!(ping_bytes.len(), Header::SIZE);

        let pong_bytes = Pong.encode();
        assert_eq!(pong_bytes.len(), Header::SIZE);
    }

    #[test]
    fn test_response_encode_decode() {
        let resp = Response::success();
        let bytes = resp.encode();
        assert!(bytes.len() > Header::SIZE);

        let mut body = Bytes::copy_from_slice(&bytes[Header::SIZE..]);
        let decoded = Response::decode(&mut body).unwrap();
        assert_eq!(decoded.code, 0);
        assert_eq!(decoded.message, "success");
    }

    #[test]
    fn test_connect_request_roundtrip() {
        let req = ConnectRequest {
            id: vec![1, 2, 3, 4, 5],
        };
        let bytes = req.encode();
        let mut body = Bytes::copy_from_slice(&bytes[Header::SIZE..]);
        let decoded = ConnectRequest::decode(&mut body).unwrap();
        assert_eq!(decoded.id, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_join_session_request_roundtrip() {
        let req = JoinSessionRequest {
            key: vec![0xAB; 32],
        };
        let bytes = req.encode();
        let mut body = Bytes::copy_from_slice(&bytes[Header::SIZE..]);
        let decoded = JoinSessionRequest::decode(&mut body).unwrap();
        assert_eq!(decoded.key, vec![0xAB; 32]);
    }

    #[test]
    fn test_session_invitation_roundtrip() {
        let inv = SessionInvitation {
            from: b"device-id-123".to_vec(),
            key: b"session-key-456".to_vec(),
            address: b"192.168.1.1".to_vec(),
            port: 22067,
            server_socket: true,
        };
        let bytes = inv.encode();
        let mut body = Bytes::copy_from_slice(&bytes[Header::SIZE..]);
        let decoded = SessionInvitation::decode(&mut body).unwrap();
        assert_eq!(decoded.from, b"device-id-123");
        assert_eq!(decoded.key, b"session-key-456");
        assert_eq!(decoded.address, b"192.168.1.1");
        assert_eq!(decoded.port, 22067);
        assert!(decoded.server_socket);
    }

    #[test]
    fn test_opaque_padding() {
        // 3 字节数据应该填充到 4 字节
        let mut buf = BytesMut::new();
        encode_opaque(&mut buf, &[1, 2, 3]);
        assert_eq!(buf.len(), 4 + 3 + 1); // len(4) + data(3) + pad(1)

        let mut bytes = buf.freeze();
        let decoded = decode_opaque(&mut bytes).unwrap();
        assert_eq!(decoded, vec![1, 2, 3]);
        assert!(bytes.is_empty());
    }

    #[tokio::test]
    async fn test_read_write_message() {
        let mut buf: Vec<u8> = Vec::new();
        let msg = Message::Ping(Ping);
        write_message(&mut buf, &msg).await.unwrap();
        assert_eq!(buf.len(), Header::SIZE);

        let mut cursor = std::io::Cursor::new(buf);
        let read_back = read_message(&mut cursor).await.unwrap();
        matches!(read_back, Message::Ping(_));
    }
}
