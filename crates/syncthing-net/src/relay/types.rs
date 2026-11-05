//! Relay protocol type definitions and error types.

/// Relay client error.
#[derive(Debug)]
pub enum RelayError {
    /// Protocol-level error (codec, illegal message, etc.).
    Protocol(String),
    /// Connection closed.
    ConnectionClosed,
    /// Rejected by the server.
    Rejected(String),
    /// Relay is full.
    RelayFull,
    /// Peer device is not registered on the relay.
    PeerNotFound,
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(s) => write!(f, "relay protocol error: {}", s),
            Self::ConnectionClosed => write!(f, "relay connection closed"),
            Self::Rejected(s) => write!(f, "relay rejected: {}", s),
            Self::RelayFull => write!(f, "relay full"),
            Self::PeerNotFound => write!(f, "peer not found on relay"),
        }
    }
}

impl std::error::Error for RelayError {}

impl From<RelayError> for syncthing_core::SyncthingError {
    fn from(e: RelayError) -> Self {
        syncthing_core::SyncthingError::network(e.to_string())
    }
}

/// Result of a relay operation.
pub type Result<T> = std::result::Result<T, RelayError>;
