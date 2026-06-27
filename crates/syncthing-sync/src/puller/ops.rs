//! Puller 辅助操作
//!
//! 内容恢复、本地复制源查找、内容哈希计算。

use crate::database::LocalDatabase;
use crate::error::Result;
use sha2::Digest;
use std::path::{Path, PathBuf};
use syncthing_core::types::{FileInfo, FileInfoBase};
use syncthing_versioner::Versioner;
use tokio::fs;
use tracing::{debug, trace, warn};

/// 计算字节数组的内容哈希 (SHA-256)
pub(crate) fn content_hash(data: &[u8]) -> Vec<u8> {
    sha2::Sha256::digest(data).to_vec()
}

/// 从 versioner 归档中恢复 base 版本内容
///
/// 通过 content_hash 匹配，而不是文件修改时间，因为归档文件的 mtime
/// 通常由 archive 时刻决定，不一定与原始 base 版本相同。
pub(crate) async fn recover_base_content(
    versioner: &dyn Versioner,
    file_path: &Path,
    base: &FileInfoBase,
) -> Option<String> {
    let expected_hash = base.content_hash.as_ref()?;
    trace!(?expected_hash, "Recovering base content from versioner");
    let versions = versioner.get_versions(file_path).await.ok()?;
    debug!(
        version_count = versions.len(),
        "Candidate versions for base recovery"
    );
    for version in versions {
        trace!(
            version_time = ?version.version_time,
            size = version.size,
            "Checking version candidate"
        );
        // 使用临时目录 restore，避免覆盖本地文件
        let tmp_dir = std::env::temp_dir().join(format!(
            "syncthing-base-{}-{:x}",
            std::process::id(),
            version
                .version_time
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        let _ = std::fs::create_dir_all(&tmp_dir);
        let file_name = file_path.file_name().and_then(|n| n.to_str())?;
        let tmp_path = tmp_dir.join(file_name);
        trace!(tmp_path = %tmp_path.display(), "Attempting restore to temp path");
        match versioner.restore(&tmp_path, version.version_time).await {
            Ok(()) => {
                if let Ok(content) = fs::read(&tmp_path).await {
                    let hash = sha2::Sha256::digest(&content).to_vec();
                    trace!(
                        restored_hash = ?hash,
                        expected_hash = ?expected_hash,
                        "Hash comparison for base recovery"
                    );
                    if hash == *expected_hash {
                        let content = String::from_utf8(content).ok();
                        let _ = std::fs::remove_dir_all(&tmp_dir);
                        return content;
                    }
                }
                let _ = std::fs::remove_dir_all(&tmp_dir);
            }
            Err(e) => {
                warn!(error = %e, "Restore failed for base recovery candidate");
                let _ = std::fs::remove_dir_all(&tmp_dir);
            }
        }
    }
    None
}

/// 查找本地具有相同块哈希的文件（重命名优化）
///
/// 如果本地已有与目标文件块哈希相同的文件，可以作为复制源，
/// 避免从远程重新下载整个文件。
pub(crate) async fn find_local_copy_source(
    folder_path: &Path,
    file_info: &FileInfo,
    db: &dyn LocalDatabase,
    folder_id: &str,
) -> Result<Option<PathBuf>> {
    if file_info.blocks.is_empty() || file_info.is_deleted() {
        return Ok(None);
    }

    let db_files = db.get_folder_files(folder_id).await?;
    for db_file in db_files {
        if db_file.is_deleted() || db_file.name == file_info.name {
            continue;
        }
        if db_file.blocks.len() != file_info.blocks.len() {
            continue;
        }
        let same_blocks = db_file
            .blocks
            .iter()
            .zip(file_info.blocks.iter())
            .all(|(a, b)| a.hash == b.hash);
        if same_blocks {
            let source_path = folder_path.join(&db_file.name);
            if source_path.exists() && source_path.is_file() {
                return Ok(Some(source_path));
            }
        }
    }
    Ok(None)
}
