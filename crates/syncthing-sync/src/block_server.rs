//! Block upload service.
//!
//! Serves block read requests issued by remote peers via BEP Request messages,
//! with path-traversal validation, hash verification, temporary-file support,
//! and standard BEP error-code mapping.

use std::path::{Path, PathBuf};

use bep_protocol::messages::{ErrorCode, Request};
use sha2::{Digest, Sha256};
use tokio::task;

/// Errors while serving a block request, mapped directly to BEP `ErrorCode`.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockRequestError {
    /// The folder does not exist. Maps to `ErrorCode::NoSuchFile`.
    FolderNotFound,
    /// The file does not exist. Maps to `ErrorCode::NoSuchFile`.
    FileNotFound,
    /// The file name is invalid (e.g. path traversal). Maps to `ErrorCode::InvalidFile`.
    InvalidFileName,
    /// The block hash does not match. Maps to `ErrorCode::InvalidFile`.
    HashMismatch,
    /// Any other I/O failure. Maps to `ErrorCode::Generic`.
    IoError(String),
}

impl BlockRequestError {
    /// Returns the BEP `ErrorCode` corresponding to this error.
    pub fn error_code(&self) -> ErrorCode {
        match self {
            BlockRequestError::FolderNotFound => ErrorCode::NoSuchFile,
            BlockRequestError::FileNotFound => ErrorCode::NoSuchFile,
            BlockRequestError::InvalidFileName => ErrorCode::InvalidFile,
            BlockRequestError::HashMismatch => ErrorCode::InvalidFile,
            BlockRequestError::IoError(_) => ErrorCode::Generic,
        }
    }
}

impl std::fmt::Display for BlockRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockRequestError::FolderNotFound => write!(f, "folder not found"),
            BlockRequestError::FileNotFound => write!(f, "file not found"),
            BlockRequestError::InvalidFileName => write!(f, "invalid file name"),
            BlockRequestError::HashMismatch => write!(f, "block hash mismatch"),
            BlockRequestError::IoError(msg) => write!(f, "io error: {}", msg),
        }
    }
}

impl std::error::Error for BlockRequestError {}

/// Validates a remote file `name` string without building a `PathBuf`.
///
/// BEP names use `/` as the separator. Rejects:
/// - empty strings
/// - names starting with `/` or `\\` (absolute paths)
/// - names containing `\0` (NUL bytes)
/// - names containing `..` or `.` segments (parent-directory traversal)
/// - names containing empty segments or backslashes within a segment
///
/// SAFETY: this must be called on every remote-received `file_info.name`
/// to prevent path-traversal attacks from writing outside the sync folder.
pub(crate) fn validate_remote_name(name: &str) -> Result<(), BlockRequestError> {
    if name.is_empty() {
        return Err(BlockRequestError::InvalidFileName);
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(BlockRequestError::InvalidFileName);
    }
    if name.contains('\0') {
        return Err(BlockRequestError::InvalidFileName);
    }
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(BlockRequestError::InvalidFileName);
        }
        if segment.contains('\\') {
            return Err(BlockRequestError::InvalidFileName);
        }
    }
    Ok(())
}

/// Validates and normalizes the `name` segments of a file path.
///
/// Ensures the BEP `/`-separated name contains no `..` segments, is not
/// absolute, and contains no empty or NUL-containing segments.
fn sanitize_name(name: &str) -> Result<PathBuf, BlockRequestError> {
    validate_remote_name(name)?;
    let mut result = PathBuf::new();
    for segment in name.split('/') {
        result.push(segment);
    }
    Ok(result)
}

/// Serves a single block request.
///
/// Performs the file read and hash verification in a blocking task and
/// returns the block data or the corresponding [`BlockRequestError`].
///
/// # Errors
///
/// Returns [`BlockRequestError`] when the name is invalid, the file is
/// missing, I/O fails, or a supplied block hash does not match.
pub async fn serve_block_request(
    folder_root: &Path,
    req: &Request,
) -> Result<Vec<u8>, BlockRequestError> {
    let folder_root = folder_root.to_path_buf();
    let req = req.clone();

    task::spawn_blocking(move || serve_block_request_sync(&folder_root, &req))
        .await
        .map_err(|e| BlockRequestError::IoError(e.to_string()))?
}

fn serve_block_request_sync(
    folder_root: &Path,
    req: &Request,
) -> Result<Vec<u8>, BlockRequestError> {
    let relative_path = sanitize_name(&req.name)?;

    let final_path = folder_root.join(&relative_path);

    let file_path = if req.from_temporary {
        let parent = final_path.parent().unwrap_or(folder_root);
        let file_name = final_path
            .file_name()
            .ok_or(BlockRequestError::InvalidFileName)?
            .to_str()
            .ok_or(BlockRequestError::InvalidFileName)?;
        parent.join(format!(".syncthing.{}.tmp", file_name))
    } else {
        final_path
    };

    // Defense in depth: re-verify the canonicalized path stays under
    // folder_root, guarding against symlink attacks even if name validation
    // was bypassed. When the file does not exist yet, canonicalize fails and
    // a normalized absolute path is used for the prefix check instead.
    let canonical_root =
        std::fs::canonicalize(folder_root).unwrap_or_else(|_| folder_root.to_path_buf());
    let canonical_file = match std::fs::canonicalize(&file_path) {
        Ok(p) => p,
        Err(_) => {
            if file_path.is_absolute() {
                file_path
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(&file_path)
            }
        }
    };

    if !canonical_file.starts_with(&canonical_root) {
        return Err(BlockRequestError::InvalidFileName);
    }

    let mut file = std::fs::File::open(&canonical_file).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => BlockRequestError::FileNotFound,
        _ => BlockRequestError::IoError(e.to_string()),
    })?;

    use std::io::{Read, Seek};
    file.seek(std::io::SeekFrom::Start(req.offset as u64))
        .map_err(|e| BlockRequestError::IoError(e.to_string()))?;

    let size = req.size.max(0) as usize;
    let mut buf = vec![0u8; size];
    let n = file
        .read(&mut buf)
        .map_err(|e| BlockRequestError::IoError(e.to_string()))?;
    buf.truncate(n);

    if !req.hash.is_empty() {
        let actual_hash = Sha256::digest(&buf);
        if actual_hash.as_slice() != req.hash.as_slice() {
            return Err(BlockRequestError::HashMismatch);
        }
    }

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name_valid() {
        assert_eq!(
            sanitize_name("foo/bar.txt").unwrap(),
            PathBuf::from("foo").join("bar.txt")
        );
        assert_eq!(sanitize_name("bar.txt").unwrap(), PathBuf::from("bar.txt"));
    }

    #[test]
    fn test_sanitize_name_invalid() {
        assert!(sanitize_name("../etc/passwd").is_err());
        assert!(sanitize_name("/etc/passwd").is_err());
        assert!(sanitize_name("foo/../bar").is_err());
        assert!(sanitize_name("foo/./bar").is_err());
        assert!(sanitize_name("foo//bar").is_err());
        assert!(sanitize_name("foo\\bar").is_err());
        assert!(sanitize_name("").is_err());
        assert!(sanitize_name("foo\0bar").is_err());
    }

    // ── validate_remote_name 测试 (v3.0.4 路径穿越防护) ──

    #[test]
    fn test_validate_remote_name_valid() {
        assert!(validate_remote_name("foo/bar.txt").is_ok());
        assert!(validate_remote_name("bar.txt").is_ok());
        assert!(validate_remote_name("deep/nested/path/file.rs").is_ok());
        assert!(validate_remote_name("a").is_ok());
        assert!(validate_remote_name("dir/file-name_v1.0.tar.gz").is_ok());
    }

    #[test]
    fn test_validate_remote_name_rejects_traversal() {
        assert!(validate_remote_name("../etc/passwd").is_err());
        assert!(validate_remote_name("foo/../../bar").is_err());
        assert!(validate_remote_name("./config").is_err());
        assert!(validate_remote_name(".").is_err());
        assert!(validate_remote_name("..").is_err());
        assert!(validate_remote_name("foo/..").is_err());
    }

    #[test]
    fn test_validate_remote_name_rejects_absolute() {
        assert!(validate_remote_name("/etc/passwd").is_err());
        assert!(validate_remote_name("\\windows\\system32").is_err());
        assert!(validate_remote_name("C:\\foo").is_err());
    }

    #[test]
    fn test_validate_remote_name_rejects_special() {
        assert!(validate_remote_name("").is_err());
        assert!(validate_remote_name("foo\0bar").is_err());
        assert!(validate_remote_name("foo//bar").is_err());
        assert!(validate_remote_name("foo\\bar").is_err());
    }

    #[test]
    fn test_validate_remote_name_rejects_backslash_in_component() {
        // 单个组件中包含反斜杠
        assert!(validate_remote_name("foo\\bar/baz").is_err());
    }
}
