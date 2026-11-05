//! Rename detection.
//!
//! Identifies file renames by comparing block hashes, improving sync
//! efficiency (the receiver can copy content from the old file).

use syncthing_core::types::FileInfo;
use tracing::info;

/// Returns `true` when two files have identical block hash lists.
pub(crate) fn has_same_blocks(a: &FileInfo, b: &FileInfo) -> bool {
    if a.blocks.len() != b.blocks.len() || a.blocks.is_empty() {
        return false;
    }
    a.blocks
        .iter()
        .zip(b.blocks.iter())
        .all(|(a_blk, b_blk)| a_blk.hash == b_blk.hash)
}

/// Detects renames within `changed_files` and reorders the list.
///
/// When a new file's block hashes exactly match a deleted file, it is treated
/// as a rename. New files are moved to the front of the list so the receiver
/// creates the new file first (copying content from the old file) and deletes
/// the old file afterwards.
pub(crate) fn detect_and_reorder_renames(changed_files: Vec<FileInfo>) -> Vec<FileInfo> {
    let mut rename_targets: Vec<usize> = Vec::new();

    for (idx, file) in changed_files.iter().enumerate() {
        if file.is_deleted() || file.blocks.is_empty() {
            continue;
        }
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

    // New (renamed) files first; everything else keeps its original order.
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
