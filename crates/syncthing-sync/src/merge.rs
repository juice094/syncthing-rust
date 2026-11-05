//! Three-way text merge.
//!
//! Provides a true base/local/remote three-way merge without depending on
//! git. Non-overlapping edits are merged automatically; overlapping edits
//! produce git-style conflict markers.

use similar::TextDiff;

/// Result of a three-way merge.
#[derive(Debug, Clone)]
pub struct MergeResult {
    /// Merged content.
    pub content: String,
    /// Whether the output contains conflict markers.
    pub has_conflicts: bool,
    /// Number of conflict regions.
    pub conflict_count: usize,
}

/// Splits a string into lines, keeping the trailing newline of each line.
fn split_lines(s: &str) -> Vec<String> {
    s.split_inclusive('\n').map(|s| s.to_string()).collect()
}

/// An edited region of the base content on one side of the merge.
#[derive(Debug, Clone)]
struct Edit {
    /// Start line in base (inclusive).
    start: usize,
    /// End line in base (exclusive).
    end: usize,
    /// Replacement lines, each with its trailing newline.
    lines: Vec<String>,
}

impl Edit {
    /// Builds the edit list for one side from a base-vs-side diff.
    fn from_diff(diff: &TextDiff<'_, '_, '_, str>, side: &str) -> Vec<Self> {
        let side_lines = split_lines(side);
        let mut edits = Vec::new();
        for op in diff.ops() {
            let old_range = op.old_range();
            let new_range = op.new_range();
            if old_range == new_range && matches!(op, similar::DiffOp::Equal { .. }) {
                continue;
            }
            let lines = side_lines[new_range.start..new_range.end].to_vec();
            edits.push(Self {
                start: old_range.start,
                end: old_range.end,
                lines,
            });
        }
        edits
    }
}

/// Finds overlapping local/remote edit regions and extends them to conflict regions.
///
/// Overlap of half-open ranges `[a, b)` and `[c, d)` is `a < d && c < b`;
/// overlapping pairs are then merged into maximal contiguous regions.
fn find_conflict_regions(local_edits: &[Edit], remote_edits: &[Edit]) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    for le in local_edits {
        for re in remote_edits {
            if le.start < re.end && re.start < le.end {
                regions.push((le.start.min(re.start), le.end.max(re.end)));
            }
        }
    }
    regions.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in regions {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

/// Builds the output for one side of a conflict region.
///
/// Applies the side's edit if it covers the region; otherwise uses the base content.
fn build_side(base_lines: &[String], edits: &[Edit], start: usize, end: usize) -> String {
    if let Some(edit) = edits.iter().find(|e| e.start >= start && e.start < end) {
        let mut s = String::new();
        for line in &base_lines[start..edit.start] {
            s.push_str(line);
        }
        for line in &edit.lines {
            s.push_str(line);
        }
        for line in &base_lines[edit.end..end] {
            s.push_str(line);
        }
        s
    } else {
        base_lines[start..end].concat()
    }
}

/// Performs a true three-way merge of text content.
///
/// Strategy:
/// 1. `local == remote` → return as-is.
/// 2. Only one side changed → take the changed side.
/// 3. Both sides changed:
///    - compute the base→local and base→remote edits;
///    - apply non-overlapping edits in order;
///    - emit git-style conflict markers for overlapping edits.
///
/// Insert edits have `start == end` and consume no base lines, so the output
/// position is left unchanged for them.
pub fn three_way_merge(base: &str, local: &str, remote: &str, _item_name: &str) -> MergeResult {
    if local == remote {
        return MergeResult {
            content: local.to_string(),
            has_conflicts: false,
            conflict_count: 0,
        };
    }

    if local == base {
        return MergeResult {
            content: remote.to_string(),
            has_conflicts: false,
            conflict_count: 0,
        };
    }
    if remote == base {
        return MergeResult {
            content: local.to_string(),
            has_conflicts: false,
            conflict_count: 0,
        };
    }

    let base_lines = split_lines(base);
    let base_len = base_lines.len();

    let diff_local = TextDiff::from_lines(base, local);
    let diff_remote = TextDiff::from_lines(base, remote);

    let local_edits = Edit::from_diff(&diff_local, local);
    let remote_edits = Edit::from_diff(&diff_remote, remote);

    let conflicts = find_conflict_regions(&local_edits, &remote_edits);

    let mut merged = String::new();
    let mut has_conflicts = false;
    let mut conflict_count = 0;

    let mut pos = 0usize;
    let mut li = 0usize;
    let mut ri = 0usize;
    let mut ci = 0usize;

    while pos < base_len || li < local_edits.len() || ri < remote_edits.len() {
        if let Some(&(cstart, cend)) = conflicts.get(ci) {
            if cstart == pos {
                let local_side = build_side(&base_lines, &local_edits, cstart, cend);
                let remote_side = build_side(&base_lines, &remote_edits, cstart, cend);
                merged.push_str("<<<<<<< local\n");
                merged.push_str(&local_side);
                merged.push_str("=======\n");
                merged.push_str(&remote_side);
                merged.push_str(">>>>>>> remote\n");
                has_conflicts = true;
                conflict_count += 1;
                pos = cend;
                ci += 1;
                // Skip edits covered by the conflict region.
                while li < local_edits.len() && local_edits[li].end <= pos {
                    li += 1;
                }
                while ri < remote_edits.len() && remote_edits[ri].end <= pos {
                    ri += 1;
                }
                continue;
            }
        }

        if let Some(edit) = local_edits.get(li) {
            if edit.start == pos {
                for line in &edit.lines {
                    merged.push_str(line);
                }
                if edit.start != edit.end {
                    pos = edit.end;
                }
                li += 1;
                continue;
            }
        }

        if let Some(edit) = remote_edits.get(ri) {
            if edit.start == pos {
                for line in &edit.lines {
                    merged.push_str(line);
                }
                if edit.start != edit.end {
                    pos = edit.end;
                }
                ri += 1;
                continue;
            }
        }

        let next_conflict_start = conflicts.get(ci).map(|c| c.0);
        let next_local_start = local_edits.get(li).map(|e| e.start);
        let next_remote_start = remote_edits.get(ri).map(|e| e.start);

        let next_event = [next_conflict_start, next_local_start, next_remote_start]
            .iter()
            .filter_map(|&x| x)
            .min()
            .unwrap_or(base_len)
            .max(pos);

        for line in &base_lines[pos..next_event] {
            merged.push_str(line);
        }
        pos = next_event;
    }

    MergeResult {
        content: merged,
        has_conflicts,
        conflict_count,
    }
}

/// Performs a simplified two-way merge (no base version).
///
/// Backward-compatible fallback for callers without a base revision: uses
/// `local` as the base, degrading to a two-way diff. Conflict detection is
/// coarse in this mode; prefer `three_way_merge` when a base is available.
pub fn merge_text(local: &str, remote: &str, _item_name: &str) -> MergeResult {
    three_way_merge(local, local, remote, _item_name)
}

/// Returns `true` when the path looks like a mergeable text file.
pub fn is_mergeable_text(path: &str) -> bool {
    path.ends_with(".md")
        || path.ends_with(".txt")
        || path.ends_with(".rs")
        || path.ends_with(".toml")
        || path.ends_with(".json")
        || path.ends_with(".yaml")
        || path.ends_with(".yml")
        || path.ends_with(".py")
        || path.ends_with(".js")
        || path.ends_with(".ts")
        || path.ends_with(".css")
        || path.ends_with(".html")
        || path.ends_with(".sh")
        || path.ends_with(".ps1")
        || path.ends_with(".ini")
        || path.ends_with(".cfg")
        || path.ends_with(".log")
        || path.ends_with(".c")
        || path.ends_with(".h")
        || path.ends_with(".cpp")
        || path.ends_with(".go")
        || path.ends_with(".java")
        || path.ends_with(".kt")
        || path.ends_with(".swift")
        || path.ends_with(".rb")
        || path.ends_with(".php")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_content() {
        let result = three_way_merge(
            "line1\nline2\n",
            "line1\nline2\n",
            "line1\nline2\n",
            "test.md",
        );
        assert!(!result.has_conflicts);
        assert_eq!(result.conflict_count, 0);
        assert_eq!(result.content, "line1\nline2\n");
    }

    #[test]
    fn test_local_unchanged_use_remote() {
        let result = three_way_merge(
            "line1\nline2\n",
            "line1\nline2\n",
            "line1\nline2\nline3\n",
            "test.md",
        );
        assert!(!result.has_conflicts);
        assert_eq!(result.content, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_remote_unchanged_use_local() {
        let result = three_way_merge(
            "line1\nline2\n",
            "line1\nline2\nline3\n",
            "line1\nline2\n",
            "test.md",
        );
        assert!(!result.has_conflicts);
        assert_eq!(result.content, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_non_overlapping_additions() {
        let base = "line1\nline2\n";
        let local = "line1\nlocal-add\nline2\n";
        let remote = "line1\nline2\nremote-add\n";
        let result = three_way_merge(base, local, remote, "test.md");
        assert!(!result.has_conflicts);
        assert!(result.content.contains("local-add"));
        assert!(result.content.contains("remote-add"));
    }

    #[test]
    fn test_overlapping_line_modification() {
        let base = "core: autonomy\n";
        let local = "core: collaboration\n";
        let remote = "core: synergy\n";
        let result = three_way_merge(base, local, remote, "SOUL.md");
        assert!(result.has_conflicts);
        assert_eq!(result.conflict_count, 1);
        assert!(result.content.contains("<<<<<<< local"));
        assert!(result.content.contains("core: collaboration"));
        assert!(result.content.contains("core: synergy"));
        assert!(result.content.contains(">>>>>>> remote"));
    }

    #[test]
    fn test_local_delete_remote_keep() {
        let base = "line1\nline2\n";
        let local = "line2\n";
        let remote = "line1\nline2\n";
        let result = three_way_merge(base, local, remote, "test.md");
        assert!(!result.has_conflicts);
        assert_eq!(result.content, "line2\n");
    }

    #[test]
    fn test_local_delete_remote_modify() {
        let base = "line1\nline2\n";
        let local = "line2\n";
        let remote = "line1-changed\nline2\n";
        let result = three_way_merge(base, local, remote, "test.md");
        assert!(result.has_conflicts);
        assert_eq!(result.conflict_count, 1);
    }

    #[test]
    fn test_merge_text_backward_compatible() {
        // merge_text 使用 local 作为 base，行为与旧实现接近
        let local = "line1\nline2\n";
        let remote = "line1\nline2\nline3\n";
        let result = merge_text(local, remote, "test.md");
        assert!(!result.has_conflicts);
        assert_eq!(result.content, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_is_mergeable_text() {
        assert!(is_mergeable_text("SOUL.md"));
        assert!(is_mergeable_text("config.toml"));
        assert!(is_mergeable_text("script.py"));
        assert!(is_mergeable_text("main.c"));
        assert!(is_mergeable_text("lib.h"));
        assert!(!is_mergeable_text("image.png"));
        assert!(!is_mergeable_text("binary.dll"));
    }
}
