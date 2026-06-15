use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListDirection, ListItem},
    Frame,
};

use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets::log_line::colored_log_line;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let (items, _selected_visible) = build_visible_items(app, &app.theme);

    let title = build_title(app);
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .direction(ListDirection::BottomToTop);
    f.render_widget(list, area);
}

/// 根据搜索/滚动状态构造当前可见的 ListItem 列表。
/// 返回的第二个元素是高亮项在可见列表中的索引（如适用）。
fn build_visible_items<'a>(app: &'a App, theme: &Theme) -> (Vec<ListItem<'a>>, Option<usize>) {
    let pattern = app.log_search.as_deref().filter(|p| !p.is_empty());
    match pattern {
        Some(pattern) => build_search_items(app, theme, pattern),
        None => {
            let items: Vec<ListItem<'a>> = app
                .log_lines
                .iter()
                .rev()
                .skip(app.log_scroll_offset)
                .map(|line| make_item(line, theme, false, None))
                .collect();
            (items, None)
        }
    }
}

fn build_search_items<'a>(
    app: &'a App,
    theme: &Theme,
    pattern: &str,
) -> (Vec<ListItem<'a>>, Option<usize>) {
    let matches = &app.log_search_matches;
    let total = matches.len();
    if total == 0 {
        return (Vec::new(), None);
    }

    // matches 按从旧到新排序；显示时反转，从最新匹配开始。
    let selected_rev = total.saturating_sub(1 + app.log_search_selected);
    let selected_visible = selected_rev.saturating_sub(app.log_scroll_offset);

    let items: Vec<ListItem<'a>> = matches
        .iter()
        .rev()
        .skip(app.log_scroll_offset)
        .filter_map(|&idx| app.log_lines.get(idx))
        .enumerate()
        .map(|(i, line)| make_item(line, theme, i == selected_visible, Some(pattern)))
        .collect();

    (items, Some(selected_visible))
}

fn make_item<'a>(
    line: &'a str,
    theme: &Theme,
    highlight: bool,
    pattern: Option<&str>,
) -> ListItem<'a> {
    let mut line = colored_log_line(line, theme);

    // 搜索模式下高亮匹配的子串。
    if let Some(pattern) = pattern {
        line = highlight_matches(line, pattern, theme);
    }

    // 当前选中的匹配行使用反色背景高亮。
    if highlight {
        for span in &mut line.spans {
            span.style = span
                .style
                .patch(Style::default().bg(theme.primary).fg(theme.text_primary));
        }
    }
    ListItem::new(line)
}

/// 在已着色的日志行中高亮所有匹配子串。
///
/// 实现方式：按 pattern 拆分每个 span 的文本，把匹配部分用高亮样式包裹。
fn highlight_matches<'a>(line: Line<'a>, pattern: &str, theme: &Theme) -> Line<'a> {
    let highlight_style = Style::default()
        .fg(theme.warning)
        .add_modifier(ratatui::style::Modifier::BOLD);
    let mut new_spans: Vec<Span<'a>> = Vec::new();

    for span in line.spans {
        let text = span.content.as_ref();
        let mut start = 0;
        // 简单线性扫描，查找所有不重叠匹配。
        while let Some(pos) = text[start..].to_lowercase().find(&pattern.to_lowercase()) {
            let absolute_pos = start + pos;
            let end = absolute_pos + pattern.len();

            if absolute_pos > start {
                new_spans.push(Span::styled(
                    text[start..absolute_pos].to_string(),
                    span.style,
                ));
            }
            new_spans.push(Span::styled(
                text[absolute_pos..end].to_string(),
                highlight_style,
            ));
            start = end;
            if start >= text.len() {
                break;
            }
        }
        if start < text.len() {
            new_spans.push(Span::styled(text[start..].to_string(), span.style));
        }
    }

    Line::from(new_spans)
}

fn build_title(app: &App) -> String {
    let base = "Logs (j/k or ↑↓: scroll, /: search, n/N: next/prev match, Esc: clear)";
    if let Some(pattern) = &app.log_search {
        format!(
            "{} [{}/{}] '{}'",
            base,
            app.log_search_selected + 1,
            app.log_search_matches.len(),
            pattern
        )
    } else {
        base.to_string()
    }
}
