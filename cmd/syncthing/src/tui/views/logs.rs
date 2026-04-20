use ratatui::{
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::App;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let logs: Text = app
        .log_lines
        .iter()
        .map(|l| Line::raw(l.as_str()))
        .collect::<Vec<_>>()
        .into();

    let para = Paragraph::new(logs)
        .block(Block::default().borders(Borders::ALL).title("Logs"))
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}
