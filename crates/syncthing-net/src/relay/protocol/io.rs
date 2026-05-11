//! Relay protocol async I/O helpers.

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{
    ConnectRequest, Header, JoinRelayRequest, JoinSessionRequest, Message, MessageType, Ping, Pong,
    RelayFull, Response, SessionInvitation, MAGIC,
};

/// Read a complete message from an async reader.
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

/// Write a complete message to an async writer.
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
