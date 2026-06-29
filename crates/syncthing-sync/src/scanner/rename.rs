//! 重命名检测
//!
//! 通过块哈希比较识别文件重命名操作，优化同步效率。

use syncthing_core::types::FileInfo;
use tracing::info;

/// 判断两个文件是否具有完全相同的块哈希
pub(crate) fn has_same_blocks(a: &FileInfo, b: &FileInfo) -> bool {
    if a.blocks.len() != b.blocks.len() || a.blocks.is_empty() {
        return false;
    }
    a.blocks
        .iter()
        .zip(b.blocks.iter())
        .all(|(a_blk, b_blk)| a_blk.hash == b_blk.hash)
}

/// 检测重命名操作并重新排序 changed_files
///
/// 当新文件的块哈希与某个已删除文件完全匹配时，识别为重命名。
/// 将新文件移到列表前面，确保接收端先创建新文件（可从旧文件复制内容），
/// 再删除旧文件。
pub(crate) fn detect_and_reorder_renames(changed_files: Vec<FileInfo>) -> Vec<FileInfo> {
    let mut rename_targets: Vec<usize> = Vec::new();

    for (idx, file) in changed_files.iter().enumerate() {
        if file.is_deleted() || file.blocks.is_empty() {
            continue;
        }
        // 查找相同块哈希的已删除文件
        for (del_idx, del_file) in changed_files.iter().enumerate() {
            if del_idx == idx {
                continue;
            }
            if del_file.is_deleted() && has_same_blocks(del_file, file) {
                rename_targets.push(idx);
                info!(
                    old_name = %del_file.name,
                    new_name = %file.name,
                    blocks = file.blocks.len(),
                    "Detected rename: same block hashes"
                );
                break;
            }
        }
    }

    if rename_targets.is_empty() {
        return changed_files;
    }

    // 重新排序：重命名的新文件在前，其余保持原顺序
    let mut reordered = Vec::with_capacity(changed_files.len());
    let mut added = vec![false; changed_files.len()];

    for &idx in &rename_targets {
        if !added[idx] {
            reordered.push(changed_files[idx].clone());
            added[idx] = true;
        }
    }
    for (idx, file) in changed_files.into_iter().enumerate() {
        if !added[idx] {
            reordered.push(file);
        }
    }

    reordered
}
