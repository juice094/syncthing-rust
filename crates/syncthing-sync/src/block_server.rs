//! 块上传服务
//!
//! 处理远程节点通过 BEP Request 发起的块读取请求。
//! 提供路径安全校验、hash 校验、临时文件支持和标准错误码映射。

use std::path::{Path, PathBuf};

use bep_protocol::messages::{ErrorCode, Request};
use sha2::{Digest, Sha256};
use tokio::task;

/// 块请求错误，直接映射到 BEP ErrorCode
#[derive(Debug, Clone, PartialEq)]
pub enum BlockRequestError {
    /// 文件夹不存在 -> NoSuchFile
    FolderNotFound,
    /// 文件不存在 -> NoSuchFile
    FileNotFound,
    /// 文件名不合法（路径遍历等） -> InvalidFile
    InvalidFileName,
    /// Hash 不匹配 -> InvalidFile
    HashMismatch,
    /// 通用 I/O 错误 -> Generic
    IoError(String),
}

impl BlockRequestError {
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

/// 校验并规范化文件路径中的 name 段
///
/// BEP 的 req.name 使用 '/' 作为分隔符。我们需要确保：
/// - 不包含 '..' 段
/// - 不以 '/' 开头（绝对路径）
/// - 不包含空段或含空字节的段
fn sanitize_name(name: &str) -> Result<PathBuf, BlockRequestError> {
    if name.is_empty() {
        return Err(BlockRequestError::InvalidFileName);
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(BlockRequestError::InvalidFileName);
    }
    if name.contains('\0') {
        return Err(BlockRequestError::InvalidFileName);
    }

    let mut result = PathBuf::new();
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(BlockRequestError::InvalidFileName);
        }
        // Windows 上也可能出现反斜杠，统一拒绝
        if segment.contains('\\') {
            return Err(BlockRequestError::InvalidFileName);
        }
        result.push(segment);
    }
    Ok(result)
}

/// 处理单个块请求
///
/// 在 blocking task 中完成文件读取与 hash 校验，返回块数据或错误。
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
    // 1. 路径安全校验
    let relative_path = sanitize_name(&req.name)?;

    // 2. 确定最终路径与临时路径
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

    // 3. 二次确认实际路径在 folder_root 下（防御目录穿越的绝对路径或符号链接攻击）
    let canonical_root = std::fs::canonicalize(folder_root)
        .unwrap_or_else(|_| folder_root.to_path_buf());
    let canonical_file = match std::fs::canonicalize(&file_path) {
        Ok(p) => p,
        Err(_) => {
            // 如果文件不存在，canonicalize 会失败。此时我们退而用规范化后的绝对路径比较前缀
            let abs = if file_path.is_absolute() {
                file_path.clone()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(&file_path)
            };
            abs
        }
    };

    if !canonical_file.starts_with(&canonical_root) {
        return Err(BlockRequestError::InvalidFileName);
    }

    // 4. 打开文件
    let mut file = std::fs::File::open(&canonical_file)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => BlockRequestError::FileNotFound,
            _ => BlockRequestError::IoError(e.to_string()),
        })?;

    // 5. Seek
    use std::io::{Read, Seek};
    file.seek(std::io::SeekFrom::Start(req.offset as u64))
        .map_err(|e| BlockRequestError::IoError(e.to_string()))?;

    // 6. 读取
    let size = req.size.max(0) as usize;
    let mut buf = vec![0u8; size];
    let n = file
        .read(&mut buf)
        .map_err(|e| BlockRequestError::IoError(e.to_string()))?;
    buf.truncate(n);

    // 7. Hash 校验（如果请求中提供了 hash）
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
}
