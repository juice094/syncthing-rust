use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::widgets::centered_rect;

pub fn draw(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 50, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Add Folder (Tab: next, Enter: save, Esc: cancel)");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let labels = ["Folder ID:", "Path:"];
    for (i, label) in labels.iter().enumerate() {
        let text = format!(
            "{} {}",
            label,
            app.folder_form.fields.get(i).map(|s| s.as_str()).unwrap_or("")
        );
        let style = if app.folder_form.focus == i {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let para = Paragraph::new(text).style(style).block(Block::default().borders(Borders::ALL));
        f.render_widget(para, chunks[i]);
    }

    let items: Vec<ListItem> = app
        .config
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let checked = app.folder_device_selection.get(i).copied().unwrap_or(false);
            let marker = if checked { "[x]" } else { "[ ]" };
            let name = d.name.as_deref().unwrap_or("Unnamed");
            let is_highlighted =
                app.folder_form.focus == app.folder_form.fields.len() && app.folder_device_selected == i;
            let style = if is_highlighted {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(format!("{} {} - {}", marker, name, d.id.to_string()))).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Shared with devices (↑↓: move, Space: toggle)"));
    f.render_widget(list, chunks[2]);

    f.render_widget(block, area);
}
