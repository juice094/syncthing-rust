use ratatui::{
    layout::Alignment,
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let theme = &app.theme;
    let daemon_style = if app.daemon_running {
        theme.style_online
    } else {
        theme.style_offline
    };

    let f5_hint = if app.external_daemon && app.tray_client.is_none() {
        " External  "
    } else {
        " Run/Stop  "
    };

    // 状态栏内容按重要程度从左到右排列：快捷键 > daemon 状态 > 日志级别。
    // 使用左对齐，终端宽度不足时右侧自然被截断，避免居中对齐导致的
    // "中间内容可见、左右提示被切掉" 的初次显示错位问题。
    let text = Text::from(vec![Line::from(vec![
        Span::styled("F5", theme.style_header),
        Span::styled(f5_hint, theme.style_idle),
        Span::styled("Tab", theme.style_header),
        Span::styled(" Switch  ", theme.style_idle),
        Span::styled("↑↓", theme.style_header),
        Span::styled(" Navigate  ", theme.style_idle),
        Span::styled("q", theme.style_header),
        Span::styled(" Quit  ", theme.style_idle),
        Span::styled(format!("| {}", app.daemon_status), daemon_style),
        Span::styled("  l", theme.style_header),
        Span::styled(
            format!(" {} ", app.log_filter_level.as_str().to_ascii_uppercase()),
            theme.style_idle,
        ),
    ])]);

    let para = Paragraph::new(text).alignment(Alignment::Left);
    f.render_widget(para, area);
}
