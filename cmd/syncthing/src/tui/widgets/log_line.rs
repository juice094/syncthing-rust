//! 日志行着色解析器
//!
//! 解析 tracing 格式日志，提取日志级别并返回对应样式。
//! 同时高亮 span 字段（folder_id / device 等）。

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::tui::theme::Theme;

/// 检测日志级别
///
/// 优先匹配 tracing 默认格式前缀：
/// `2026-04-20T12:15:03.581593Z DEBUG ...`
/// 不匹配时回退到行首 `[LEVEL]` 格式，最后再按关键字 contains 检测。
fn detect_level(line: &str) -> &str {
    let trimmed = line.trim_start();

    // 1. tracing 默认格式：时间戳 + 级别在第二个 token
    let mut parts = trimmed.split_whitespace();
    if let (Some(ts), Some(level)) = (parts.next(), parts.next()) {
        if is_tracing_timestamp(ts) {
            match level {
                "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE" => return level,
                _ => {}
            }
        }
    }

    // 2. 回退：行首括号格式
    if trimmed.starts_with("[ERROR]") {
        return "ERROR";
    }
    if trimmed.starts_with("[WARN]") {
        return "WARN";
    }
    if trimmed.starts_with("[INFO]") {
        return "INFO";
    }
    if trimmed.starts_with("[DEBUG]") {
        return "DEBUG";
    }
    if trimmed.starts_with("[TRACE]") {
        return "TRACE";
    }

    // 3. 最后尝试关键字包含
    for level in &["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] {
        if trimmed.contains(level) {
            return level;
        }
    }

    "INFO"
}

/// 校验 tracing 默认时间戳前缀：`YYYY-MM-DDTHH:MM:SS.ssssssZ`
fn is_tracing_timestamp(ts: &str) -> bool {
    let b = ts.as_bytes();
    // 最短形式：YYYY-MM-DDTHH:MM:SS.0Z => 21 字节
    if b.len() < 21 {
        return false;
    }
    if b.last() != Some(&b'Z') {
        return false;
    }

    let expected = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    for &(pos, ch) in &expected {
        if b.get(pos) != Some(&ch) {
            return false;
        }
    }

    for &i in &[0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15] {
        if b.get(i).is_none_or(|c| !c.is_ascii_digit()) {
            return false;
        }
    }

    // 秒后面必须是 `.` + 数字
    let mut found_dot = false;
    for &c in &b[17..b.len() - 1] {
        if c == b'.' {
            if found_dot {
                return false;
            }
            found_dot = true;
            continue;
        }
        if !c.is_ascii_digit() {
            return false;
        }
    }

    found_dot
}

/// 解析并高亮 span 字段
/// 输入: `folder_id=test-folder device=IKOL33P...`
/// 输出: Spans 数组，字段名和值用不同颜色
fn highlight_spans<'a>(line: &'a str, theme: &Theme) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let mut rest = line;

    // 简单启发式：查找 `key=value` 模式
    while let Some(eq_pos) = rest.find('=') {
        let before = &rest[..eq_pos];
        let after = &rest[eq_pos + 1..];

        // 找到 key 的开始（上一个空格）
        let key_start = before.rfind(' ').map(|i| i + 1).unwrap_or(0);
        let key = &before[key_start..];

        // 找到 value 的结束（下一个空格，或引号结束）
        let (value, remaining) = if let Some(quoted) = after.strip_prefix('"') {
            if let Some(end) = quoted.find('"') {
                (&after[..end + 2], &after[end + 2..])
            } else {
                (after, "")
            }
        } else if let Some(sp) = after.find(' ') {
            (&after[..sp], &after[sp..])
        } else {
            (after, "")
        };

        // 添加 key=value 之前的文本
        if key_start > 0 {
            spans.push(Span::raw(before[..key_start].to_string()));
        }

        // 添加 key
        spans.push(Span::styled(
            format!("{}=", key),
            Style::default().fg(theme.secondary),
        ));
        // 添加 value
        spans.push(Span::styled(
            value.to_string(),
            Style::default().fg(theme.info),
        ));

        rest = remaining;
    }

    if !rest.is_empty() {
        spans.push(Span::raw(rest.to_string()));
    }

    if spans.is_empty() {
        spans.push(Span::raw(line.to_string()));
    }

    spans
}

/// 将原始日志行转换为着色的 Line
pub fn colored_log_line<'a>(line: &'a str, theme: &Theme) -> Line<'a> {
    let level = detect_level(line);
    let base_style = theme.log_level_style(level);

    // 对于 DEBUG/INFO 级别，额外高亮 span 字段
    let spans = if level == "DEBUG" || level == "INFO" {
        let mut spans = highlight_spans(line, theme);
        // 应用基础样式（如果 span 没有自己的颜色）
        for span in &mut spans {
            if span.style.fg.is_none() {
                span.style = base_style;
            }
        }
        spans
    } else {
        vec![Span::styled(line.to_string(), base_style)]
    };

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tracing_levels() {
        assert_eq!(
            detect_level("2026-04-20T12:15:03.581593Z DEBUG syncthing_sync::folder_model foo"),
            "DEBUG"
        );
        assert_eq!(
            detect_level("2026-04-20T12:15:03.581593Z ERROR something failed"),
            "ERROR"
        );
        assert_eq!(detect_level("2026-04-20T12:15:03.581593Z WARN x"), "WARN");
        assert_eq!(detect_level("2026-04-20T12:15:03.581593Z INFO x"), "INFO");
        assert_eq!(detect_level("2026-04-20T12:15:03.581593Z TRACE x"), "TRACE");
    }

    #[test]
    fn info_with_error_substring_stays_info() {
        let line = "2026-04-20T12:15:03.581593Z INFO finished handling error request";
        assert_eq!(detect_level(line), "INFO");
    }

    #[test]
    fn bracket_prefix_overrides_contains() {
        assert_eq!(detect_level("[WARN] everything is fine DEBUG"), "WARN");
        assert_eq!(detect_level("[ERROR] lower warn word"), "ERROR");
    }

    #[test]
    fn fallback_contains_level() {
        assert_eq!(detect_level("Something ERROR happened"), "ERROR");
        assert_eq!(detect_level("DEBUG log without prefix"), "DEBUG");
    }
}
