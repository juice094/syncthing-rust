use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets::centered_rect;

pub fn draw(f: &mut Frame, app: &App, theme: &Theme) {
    let area = centered_rect(60, 50, f.area());

    // Dim background
    let dim = ratatui::widgets::Block::default().style(Style::default().bg(Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());

    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused)
        .title(Span::styled(" Add Folder ", theme.style_header));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    // Folder ID field
    let id_focused = app.folder_form.focus == 0;
    let id_text = format!(
        "Folder ID: {}",
        app.folder_form.fields.first().map(|s| s.as_str()).unwrap_or("")
    );
    let id_para = Paragraph::new(id_text)
        .style(if id_focused {
            Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary)
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if id_focused { theme.border_focused } else { theme.border }),
        );
    f.render_widget(id_para, chunks[0]);

    // Path field
    let path_focused = app.folder_form.focus == 1;
    let path_text = format!(
        "Path: {}",
        app.folder_form.fields.get(1).map(|s| s.as_str()).unwrap_or("")
    );
    let path_para = Paragraph::new(path_text)
        .style(if path_focused {
            Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary)
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if path_focused { theme.border_focused } else { theme.border }),
        );
    f.render_widget(path_para, chunks[1]);

    // Hint
    let in_device_list = app.folder_form.focus == app.folder_form.fields.len();
    if id_focused {
        let hint = Paragraph::new(Span::styled("Unique identifier for this folder", theme.style_idle));
        f.render_widget(hint, chunks[2]);
    } else if path_focused {
        let hint = Paragraph::new(Span::styled("Absolute path to the local directory", theme.style_idle));
        f.render_widget(hint, chunks[2]);
    }

    // Device selection list
    let list_focused = in_device_list;
    let items: Vec<ListItem> = app
        .config
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let checked = app.folder_device_selection.get(i).copied().unwrap_or(false);
            let marker = if checked { "☑" } else { "☐" };
            let name = d.name.as_deref().unwrap_or("Unnamed");
            let is_highlighted = list_focused && app.folder_device_selected == i;
            let style = if is_highlighted {
                Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_secondary)
            };
            ListItem::new(Line::from(format!("{} {} — {}", marker, name, d.id))).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if list_focused { theme.border_focused } else { theme.border })
            .title(Span::styled(" Shared with devices ", theme.style_header)),
    );
    f.render_widget(list, chunks[3]);

    // Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab", theme.style_header),
        Span::styled(" next  ", theme.style_idle),
        Span::styled("↑↓", theme.style_header),
        Span::styled(" navigate  ", theme.style_idle),
        Span::styled("Space", theme.style_header),
        Span::styled(" toggle  ", theme.style_idle),
        Span::styled("Enter", theme.style_header),
        Span::styled(" save  ", theme.style_idle),
        Span::styled("Esc", theme.style_header),
        Span::styled(" cancel", theme.style_idle),
    ]));
    f.render_widget(footer, chunks[4]);

    f.render_widget(block, area);
}
