use ratatui::{
    layout::Alignment,
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

use crate::tui::app::App;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let daemon_color = if app.daemon_running {
        Color::Green
    } else {
        Color::Red
    };
    let status = format!("Daemon: {}", app.daemon_status);
    let text = Text::from(vec![Line::from(vec![
        Span::styled("F5: Run/Stop  ", Style::default()),
        Span::styled("Tab/←→: Switch  ", Style::default()),
        Span::styled("q: Quit  ", Style::default()),
        Span::styled(status, Style::default().fg(daemon_color)),
    ])]);
    let para = Paragraph::new(text).alignment(Alignment::Center);
    f.render_widget(para, area);
}
