use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets::centered_popup;

pub fn draw(f: &mut Frame, app: &App, theme: &Theme) {
    let area = centered_popup(60, 16, f.area());

    let dim = ratatui::widgets::Block::default().style(Style::default().bg(Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused)
        .title(Span::styled(" Edit Folder ", theme.style_header));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(area);

    // Folder ID (read-only)
    let id_focused = app.folder_form.focus == 0;
    let id_val = app
        .folder_form
        .fields
        .first()
        .map(|s| s.as_str())
        .unwrap_or("");
    let id_display = format!("Folder ID: {}", id_val);
    let id_para = Paragraph::new(id_display)
        .style(Style::default().fg(Color::DarkGray))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border),
        );
    f.render_widget(id_para, chunks[0]);

    // Path field
    let path_focused = app.folder_form.focus == 1;
    let path_val = app
        .folder_form
        .fields
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("");
    let path_display = if path_focused && path_val.is_empty() {
        "Path: ▎".to_string()
    } else {
        format!("Path: {}", path_val)
    };
    let path_para = Paragraph::new(path_display)
        .style(if path_focused {
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary)
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if path_focused {
                    theme.border_focused
                } else {
                    theme.border
                }),
        );
    f.render_widget(path_para, chunks[1]);

    // Hint
    if id_focused {
        let hint = Paragraph::new(Span::styled(
            "Folder ID cannot be changed",
            theme.style_idle,
        ));
        f.render_widget(hint, chunks[2]);
    } else if path_focused {
        let hint = Paragraph::new(Span::styled(
            "Absolute path, e.g. C:\\Users\\me\\sync",
            theme.style_idle,
        ));
        f.render_widget(hint, chunks[2]);
    }

    // Device selection list
    let list_focused = app.folder_form.focus == app.folder_form.fields.len();
    let items: Vec<ListItem> = app
        .config
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let checked = app.folder_device_selection.get(i).copied().unwrap_or(false);
            let marker = if checked { "[x]" } else { "[ ]" };
            let name = d.name.as_deref().unwrap_or("Unnamed");
            let is_highlighted = list_focused && app.folder_device_selected == i;
            let style = if is_highlighted {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_secondary)
            };
            ListItem::new(Line::from(format!("{} {} — {}", marker, name, d.id))).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if list_focused {
                theme.border_focused
            } else {
                theme.border
            })
            .title(Span::styled(" Share with ", theme.style_header)),
    );
    f.render_widget(list, chunks[3]);

    // Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab", theme.style_header),
        Span::styled(" next  ", theme.style_idle),
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
