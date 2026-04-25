//! BEP handshake helper
//!
//! Decouples the BEP Hello exchange from the underlying transport so that
//! TCP, QUIC, or in-memory pipes can reuse the same logic.

use tokio::io::{AsyncRead, AsyncWrite};
use tracing::info;

use syncthing_core::{Result, SyncthingError};

/// BEP Hello exchange utilities.
pub struct BepHandshaker;

impl BepHandshaker {
    /// Build the standard Hello message for this implementation.
    fn build_hello(device_name: &str) -> bep_protocol::messages::Hello {
        bep_protocol::messages::Hello {
            device_name: device_name.to_string(),
            client_name: "syncthing".to_string(),
            client_version: "v2.0.16".to_string(),
            num_connections: 1,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        }
    }

    /// Perform the server-side BEP Hello exchange.
    ///
    /// The server waits for the remote Hello, then responds with its own.
    /// Returns the decoded remote Hello on success.
    pub async fn server_handshake<RW>(
        stream: &mut RW,
        device_name: &str,
    ) -> Result<bep_protocol::messages::Hello>
    where
        RW: AsyncRead + AsyncWrite + Unpin,
    {
        let remote_hello = bep_protocol::handshake::recv_hello(stream)
            .await
            .map_err(|e| SyncthingError::protocol(format!("failed to receive hello: {}", e)))?;

        let our_hello = Self::build_hello(device_name);
        bep_protocol::handshake::send_hello(stream, &our_hello)
            .await
            .map_err(|e| SyncthingError::protocol(format!("failed to send hello: {}", e)))?;

        info!(
            "Incoming BEP hello exchange complete: remote_device={}",
            remote_hello.device_name
        );
        Ok(remote_hello)
    }

    /// Perform the client-side BEP Hello exchange.
    ///
    /// The client sends its Hello first, then waits for the remote one.
    /// Returns the decoded remote Hello on success.
    pub async fn client_handshake<RW>(
        stream: &mut RW,
        device_name: &str,
    ) -> Result<bep_protocol::messages::Hello>
    where
        RW: AsyncRead + AsyncWrite + Unpin,
    {
        let our_hello = Self::build_hello(device_name);
        bep_protocol::handshake::send_hello(stream, &our_hello)
            .await
            .map_err(|e| SyncthingError::protocol(format!("failed to send hello: {}", e)))?;

        let remote_hello = bep_protocol::handshake::recv_hello(stream)
            .await
            .map_err(|e| SyncthingError::protocol(format!("failed to receive hello: {}", e)))?;

        info!(
            "Outgoing BEP hello exchange complete: remote_device={}",
            remote_hello.device_name
        );
        Ok(remote_hello)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncthing_test_utils::memory_pipe_pair;

    #[tokio::test]
    async fn test_handshake_over_memory_pipe() {
        let (mut server_pipe, mut client_pipe) = memory_pipe_pair(1024);

        let server_handle = tokio::spawn(async move {
            BepHandshaker::server_handshake(&mut server_pipe, "server-device")
                .await
                .unwrap()
        });

        let client_result =
            BepHandshaker::client_handshake(&mut client_pipe, "client-device")
                .await
                .unwrap();

        let server_result = server_handle.await.unwrap();

        assert_eq!(client_result.device_name, "server-device");
        assert_eq!(server_result.device_name, "client-device");
        assert_eq!(client_result.client_name, "syncthing");
        assert_eq!(server_result.client_name, "syncthing");
    }
}
