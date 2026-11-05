//! Error types for the sync engine.

use std::io;
use std::path::PathBuf;

/// Top-level error type for sync operations.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// Underlying I/O failure.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// Error propagated from `syncthing-core`.
    #[error("Core error: {0}")]
    Core(#[from] syncthing_core::SyncthingError),

    /// Failure while scanning a folder.
    #[error("Scan error in folder '{folder}': {message}")]
    Scan { folder: String, message: String },

    /// Failure while pulling an item.
    #[error("Pull error for '{path}': {message}")]
    Pull { path: String, message: String },

    /// Database operation failure.
    #[error("Database error: {0}")]
    Database(String),

    /// Referenced folder does not exist.
    #[error("Folder not found: {0}")]
    FolderNotFound(String),

    /// Referenced file does not exist.
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    /// Index processing failure.
    #[error("Index error: {0}")]
    Index(String),

    /// Conflict handling failure.
    #[error("Conflict error for '{path}': {message}")]
    Conflict { path: String, message: String },

    /// Block checksum did not match the expected value.
    #[error("Checksum mismatch for block at offset {offset}")]
    ChecksumMismatch { offset: i64 },

    /// Operation exceeded its deadline.
    #[error("Operation timeout: {context}")]
    Timeout { context: String },

    /// Operation was cancelled.
    #[error("Operation cancelled")]
    Cancelled,

    /// Invalid configuration.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Any other error, described by the contained message.
    #[error("{0}")]
    Other(String),
}

impl SyncError {
    /// Creates a [`SyncError::Scan`] variant.
    pub fn scan<S: Into<String>>(folder: S, message: S) -> Self {
        Self::Scan {
            folder: folder.into(),
            message: message.into(),
        }
    }

    /// Creates a [`SyncError::Pull`] variant.
    pub fn pull<S: Into<String>>(path: S, message: S) -> Self {
        Self::Pull {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Creates a [`SyncError::Database`] variant.
    pub fn database<S: Into<String>>(message: S) -> Self {
        Self::Database(message.into())
    }

    /// Creates a [`SyncError::Index`] variant.
    pub fn index<S: Into<String>>(message: S) -> Self {
        Self::Index(message.into())
    }

    /// Creates a [`SyncError::Conflict`] variant.
    pub fn conflict<S: Into<String>>(path: S, message: S) -> Self {
        Self::Conflict {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Creates a [`SyncError::Timeout`] variant.
    pub fn timeout<S: Into<String>>(context: S) -> Self {
        Self::Timeout {
            context: context.into(),
        }
    }

    /// Returns `true` if the error is transient and the operation may succeed on retry.
    pub fn is_temporary(&self) -> bool {
        matches!(
            self,
            SyncError::Io(_) | SyncError::Timeout { .. } | SyncError::Pull { .. }
        )
    }
}

/// Result alias using [`SyncError`] as the error type.
pub type Result<T> = std::result::Result<T, SyncError>;
