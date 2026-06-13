//! 文本文件三路合并
//!
//! 提供基于 base / local / remote 的真正三路合并，不依赖 git。
//! 不重叠的修改自动合并，重叠修改插入 git 风格冲突标记。

use similar::TextDiff;

/// 合并结果
#[derive(Debug, Clone)]
pub struct MergeResult {
    /// 合并后的内容
    pub content: String,
    /// 是否存在冲突标记
    pub has_conflicts: bool,
    /// 冲突区域数量
    pub conflict_count: usize,
}

/// 将字符串按行分割，保留每行末尾的换行符
fn split_lines(s: &str) -> Vec<String> {
    s.split_inclusive('\n').map(|s| s.to_string()).collect()
}

/// 表示 base 中的一个编辑区间
#[derive(Debug, Clone)]
struct Edit {
    /// 在 base 中的起始行（包含）
    start: usize,
    /// 在 base 中的结束行（不包含）
    end: usize,
    /// 替换后的行（带换行符）
    lines: Vec<String>,
}

impl Edit {
    /// 从 diff 构建编辑列表
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

/// 找出 local 与 remote 编辑的重叠区域，并扩展为冲突区域
fn find_conflict_regions(local_edits: &[Edit], remote_edits: &[Edit]) -> Vec<(usize, usize)> {
    let mut regions = Vec::new();
    for le in local_edits {
        for re in remote_edits {
            // 区间重叠判断：[a,b) 与 [c,d) 重叠当且仅当 a < d && c < b
            if le.start < re.end && re.start < le.end {
                regions.push((le.start.min(re.start), le.end.max(re.end)));
            }
        }
    }
    // 合并重叠的冲突区域
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

/// 为冲突区域构建一侧的输出
///
/// 如果该侧在冲突区域内有编辑，应用编辑；否则使用 base 内容。
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

/// 对文本内容执行真正的三路合并
///
/// 策略：
/// 1. local == remote → 直接返回。
/// 2. 只有一侧发生变化 → 采用变化侧。
/// 3. 两侧都发生变化：
///    - 计算 base→local 和 base→remote 的编辑。
///    - 非重叠编辑自动顺序应用。
///    - 重叠编辑生成 git 风格冲突标记。
pub fn three_way_merge(base: &str, local: &str, remote: &str, _item_name: &str) -> MergeResult {
    // 完全相同 → 无需合并
    if local == remote {
        return MergeResult {
            content: local.to_string(),
            has_conflicts: false,
            conflict_count: 0,
        };
    }

    // 只有一侧发生变化
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
        // 当前位置是否在一个冲突区域内？
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
                // 跳过被冲突区域覆盖的 edit
                while li < local_edits.len() && local_edits[li].end <= pos {
                    li += 1;
                }
                while ri < remote_edits.len() && remote_edits[ri].end <= pos {
                    ri += 1;
                }
                continue;
            }
        }

        // 当前位置是否有 local edit？
        if let Some(edit) = local_edits.get(li) {
            if edit.start == pos {
                for line in &edit.lines {
                    merged.push_str(line);
                }
                // Insert 操作 edit.start == edit.end，不消耗 base 行，保持 pos 不变
                if edit.start != edit.end {
                    pos = edit.end;
                }
                li += 1;
                continue;
            }
        }

        // 当前位置是否有 remote edit？
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

        // 找到下一个事件位置
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

/// 对文本内容执行简化三路合并（无 base 版本）
///
/// 兼容旧接口：当没有 base 时，把 local 同时作为 base，退化为双路 diff。
/// 这种情况下的冲突判断较粗糙，建议调用方优先使用 `three_way_merge`。
pub fn merge_text(local: &str, remote: &str, _item_name: &str) -> MergeResult {
    three_way_merge(local, local, remote, _item_name)
}

/// 判断文件是否为可合并的文本类型
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
