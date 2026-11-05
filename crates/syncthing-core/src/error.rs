//! Error handling.
//!
//! Unified error type modeled on the Go version's error handling patterns.

use std::io;

/// Unified error type for syncthing-rust.
#[derive(Debug, thiserror::Error)]
pub enum SyncthingError {
    /// I/O error.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// TLS error.
    #[error("TLS error: {0}")]
    Tls(String),

    /// rustls error.
    #[error("Rustls error: {0}")]
    Rustls(String),

    /// Connection error.
    #[error("Connection error: {message}")]
    Connection { message: String },

    /// Handshake error.
    #[error("Handshake error: {message}")]
    Handshake { message: String },

    /// Protocol error.
    #[error("Protocol error: {message}")]
    Protocol { message: String },

    /// Configuration error.
    #[error("Configuration error: {message}")]
    Config { message: String },

    /// Device ID error.
    #[error("Device ID error: {message}")]
    DeviceId { message: String },

    /// Message serialization error.
    #[error("Message serialization error: {0}")]
    Serialization(String),

    /// Message parse error.
    #[error("Message parse error: {0}")]
    Parse(String),

    /// Timeout error.
    #[error("Timeout: {context}")]
    Timeout { context: String },

    /// Device not found.
    #[error("Device not found: {device_id}")]
    DeviceNotFound { device_id: String },

    /// Connection closed.
    #[error("Connection closed")]
    ConnectionClosed,

    /// Storage error.
    #[error("Storage error: {0}")]
    Storage(String),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Network error.
    #[error("Network error: {0}")]
    Network(String),

    /// Validation error.
    #[error("Validation error: {0}")]
    Validation(String),

    /// Any other error.
    #[error("{0}")]
    Other(String),
}

impl SyncthingError {
    /// Creates a connection error.
    pub fn connection<S: Into<String>>(message: S) -> Self {
        Self::Connection {
            message: message.into(),
        }
    }

    /// Creates a handshake error.
    pub fn handshake<S: Into<String>>(message: S) -> Self {
        Self::Handshake {
            message: message.into(),
        }
    }

    /// Creates a protocol error.
    pub fn protocol<S: Into<String>>(message: S) -> Self {
        Self::Protocol {
            message: message.into(),
        }
    }

    /// Creates a configuration error.
    pub fn config<S: Into<String>>(message: S) -> Self {
        Self::Config {
            message: message.into(),
        }
    }

    /// Creates a device ID error.
    pub fn device_id<S: Into<String>>(message: S) -> Self {
        Self::DeviceId {
            message: message.into(),
        }
    }

    /// Creates an I/O error from a string description.
    pub fn io<S: Into<String>>(message: S) -> Self {
        Self::Io(io::Error::other(message.into()))
    }

    /// Creates a storage error.
    pub fn storage<S: Into<String>>(message: S) -> Self {
        Self::Storage(message.into())
    }

    /// Creates an internal error.
    pub fn internal<S: Into<String>>(message: S) -> Self {
        Self::Internal(message.into())
    }

    /// Creates a network error.
    pub fn network<S: Into<String>>(message: S) -> Self {
        Self::Network(message.into())
    }

    /// Creates a timeout error.
    pub fn timeout<S: Into<String>>(context: S) -> Self {
        Self::Timeout {
            context: context.into(),
        }
    }

    /// Returns whether the error is transient (the operation is worth retrying).
    pub fn is_temporary(&self) -> bool {
        matches!(
            self,
            SyncthingError::Io(_)
                | SyncthingError::Connection { .. }
                | SyncthingError::Timeout { .. }
                | SyncthingError::ConnectionClosed
        )
    }

    /// Returns whether the error is fatal (the operation should not be retried).
    pub fn is_fatal(&self) -> bool {
        !self.is_temporary()
    }
}

/// Result alias used throughout syncthing-rust.
pub type Result<T> = std::result::Result<T, SyncthingError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_kinds() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: SyncthingError = io_err.into();
        assert!(err.is_temporary());

        let protocol_err = SyncthingError::protocol("invalid magic");
        assert!(protocol_err.is_fatal());
    }

    #[test]
    fn test_error_display() {
        let err = SyncthingError::connection("refused");
        assert_eq!(err.to_string(), "Connection error: refused");
    }
}
