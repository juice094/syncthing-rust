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
fn detect_level(line: &str) -> &str {
    // tracing 格式: "2026-04-20T12:15:03.581593Z DEBUG syncthing_sync::folder_model ..."
    // 也可能是: "[DEBUG] ..." 或 "DEBUG: ..."
    let trimmed = line.trim();

    // 尝试匹配常见级别关键字
    for level in &["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] {
        if trimmed.contains(level) {
            return level;
        }
    }

    // 回退：检查行首括号格式
    if trimmed.starts_with("[ERROR]") || trimmed.contains("error") {
        "ERROR"
    } else if trimmed.starts_with("[WARN]") || trimmed.contains("warn") {
        "WARN"
    } else if trimmed.starts_with("[INFO]") {
        "INFO"
    } else if trimmed.starts_with("[DEBUG]") {
        "DEBUG"
    } else if trimmed.starts_with("[TRACE]") {
        "TRACE"
    } else {
        "INFO"
    }
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

/// 批量转换日志行
pub fn colored_logs<'a>(lines: &'a [String], theme: &Theme) -> Vec<Line<'a>> {
    lines.iter().map(|l| colored_log_line(l, theme)).collect()
}
