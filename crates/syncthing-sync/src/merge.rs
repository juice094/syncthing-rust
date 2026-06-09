//! 文本文件三路合并
//!
//! 简化版：无 base 版本，直接对比 local vs remote。
//! 不重叠的修改自动合并，重叠修改插入 git 风格冲突标记。

use similar::{ChangeTag, TextDiff};

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

/// 对文本内容执行简化三路合并
///
/// 策略（无 base 版）：
/// 1. 将 local 和 remote 分别与 common（取 local 和 remote 的最长公共子序列近似）对比
/// 2. 更简化的做法：直接对 local 和 remote 做行级 diff
/// 3. 如果 remote 只是 local 的超集（新增行）→ 自动合并
/// 4. 如果同一行被双端修改 → 冲突标记
///
/// 实际实现：用 Myer's diff 算法比较 local↔remote，
/// 然后对每对相邻的 diff hunk 判断是否是"替换同一行"。
pub fn merge_text(local: &str, remote: &str, _item_name: &str) -> MergeResult {
    // 完全相同 → 无需合并
    if local == remote {
        return MergeResult {
            content: local.to_string(),
            has_conflicts: false,
            conflict_count: 0,
        };
    }

    let diff = TextDiff::from_lines(local, remote);
    let mut merged = String::new();
    let mut has_conflicts = false;
    let mut conflict_count = 0;

    // 收集所有 diff hunks
    let mut ops: Vec<(ChangeTag, String)> = Vec::new();
    for group in diff.grouped_ops(3) {
        for op in group {
            for change in diff.iter_changes(&op) {
                ops.push((change.tag(), change.value().to_string()));
            }
        }
    }

    // 简化策略：
    // 遍历 diff ops，将连续的 Equal + Delete + Insert 识别为"替换"
    // 如果是纯 Insert（新增行）→ 直接追加
    // 如果是 Delete + Insert（修改行）→ 检查是否为同一行的修改
    //   → 用行号判断：如果删除和插入在同一个位置附近 → 冲突标记
    //
    // 更简单的做法：直接输出 unified diff 风格的合并
    // Equal → 保留
    // Delete(local) + Insert(remote) → 如果删除的是一行，插入的也是一行 → 冲突标记
    // Insert only → 追加

    let mut i = 0;
    while i < ops.len() {
        match ops[i].0 {
            ChangeTag::Equal => {
                merged.push_str(&ops[i].1);
                i += 1;
            }
            ChangeTag::Delete => {
                // 检查下一个是否是 Insert（替换场景）
                if i + 1 < ops.len() && ops[i + 1].0 == ChangeTag::Insert {
                    // 同一行的修改 → 冲突标记
                    merged.push_str(&format!(
                        "<<<<<<< local\n{}=======\n{}>>>>>>> remote\n",
                        ops[i].1,
                        ops[i + 1].1
                    ));
                    has_conflicts = true;
                    conflict_count += 1;
                    i += 2;
                } else {
                    // 纯删除（local 有，remote 没有）→ remote 删除了这一行
                    // 在简化模型中，我们接受 remote 的版本（不保留删除的行）
                    i += 1;
                }
            }
            ChangeTag::Insert => {
                // 纯新增 → 追加 remote 的内容
                merged.push_str(&ops[i].1);
                i += 1;
            }
        }
    }

    MergeResult {
        content: merged,
        has_conflicts,
        conflict_count,
    }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_content() {
        let result = merge_text("line1\nline2\n", "line1\nline2\n", "test.md");
        assert!(!result.has_conflicts);
        assert_eq!(result.conflict_count, 0);
        assert_eq!(result.content, "line1\nline2\n");
    }

    #[test]
    fn test_non_overlapping_additions() {
        let local = "line1\nline2\n";
        let remote = "line1\nline2\nline3\n";
        let result = merge_text(local, remote, "test.md");
        assert!(!result.has_conflicts);
        assert_eq!(result.content, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_overlapping_line_modification() {
        let local = "core: autonomy\n";
        let remote = "core: collaboration\n";
        let result = merge_text(local, remote, "SOUL.md");
        assert!(result.has_conflicts);
        assert_eq!(result.conflict_count, 1);
        assert!(result.content.contains("<<<<<<< local"));
        assert!(result.content.contains("core: autonomy"));
        assert!(result.content.contains("core: collaboration"));
        assert!(result.content.contains(">>>>>>> remote"));
    }

    #[test]
    fn test_local_addition_remote_different_addition() {
        let local = "line1\nlocal-add\nline2\n";
        let remote = "line1\nremote-add\nline2\n";
        let result = merge_text(local, remote, "test.md");
        // 第二行被双端替换 → 冲突
        assert!(result.has_conflicts);
        assert_eq!(result.conflict_count, 1);
    }

    #[test]
    fn test_remote_pure_addition_no_conflict() {
        let local = "line1\n";
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
        assert!(!is_mergeable_text("image.png"));
        assert!(!is_mergeable_text("binary.dll"));
    }
}
