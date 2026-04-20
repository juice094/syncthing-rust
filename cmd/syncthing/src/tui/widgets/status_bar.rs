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

    let text = Text::from(vec![Line::from(vec![
        Span::styled("F5", theme.style_header),
        Span::styled(" Run/Stop  ", theme.style_idle),
        Span::styled("Tab", theme.style_header),
        Span::styled(" Switch  ", theme.style_idle),
        Span::styled("↑↓", theme.style_header),
        Span::styled(" Navigate  ", theme.style_idle),
        Span::styled("q", theme.style_header),
        Span::styled(" Quit  ", theme.style_idle),
        Span::styled(format!("| {}", app.daemon_status), daemon_style),
    ])]);

    let para = Paragraph::new(text).alignment(Alignment::Center);
    f.render_widget(para, area);
}
