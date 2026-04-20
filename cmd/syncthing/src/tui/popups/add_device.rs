use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::widgets::centered_rect;

pub fn draw(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 40, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Add Device (Tab: next, Enter: save, Esc: cancel)");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    let labels = ["Device ID:", "Name:", "Address (e.g. 127.0.0.1:22001):"];
    for (i, label) in labels.iter().enumerate() {
        let text = format!(
            "{} {}",
            label,
            app.device_form.fields.get(i).map(|s| s.as_str()).unwrap_or("")
        );
        let style = if app.device_form.focus == i {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let para = Paragraph::new(text).style(style).block(Block::default().borders(Borders::ALL));
        f.render_widget(para, chunks[i]);
    }

    let hint = Paragraph::new("Type to edit. Backspace to delete.").style(Style::default().fg(Color::Gray));
    f.render_widget(hint, chunks[3]);
    f.render_widget(block, area);
}
