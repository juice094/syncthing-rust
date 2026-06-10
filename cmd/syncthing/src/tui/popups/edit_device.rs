use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets::centered_popup;

pub fn draw(f: &mut Frame, app: &App, theme: &Theme) {
    let area = centered_popup(72, 14, f.area());

    let dim = ratatui::widgets::Block::default().style(Style::default().bg(Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused)
        .title(Span::styled(" Edit Device ", theme.style_header));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    let labels = [("Device ID", 0), ("Name", 1), ("Address", 2)];
    let hints = [
        "Device ID cannot be changed (read-only)",
        "Optional display name",
        "e.g. tcp://192.168.1.100:22001 (empty=dynamic)",
    ];

    for (i, (label, field_idx)) in labels.iter().enumerate() {
        let field_text = app
            .device_form
            .fields
            .get(*field_idx)
            .map(|s| s.as_str())
            .unwrap_or("");
        let is_focused = app.device_form.focus == *field_idx;
        let is_readonly = *field_idx == 0; // Device ID is read-only
        let display = if is_focused && field_text.is_empty() && !is_readonly {
            format!("{}: ▎", label)
        } else {
            format!("{}: {}", label, field_text)
        };
        let style = if is_readonly {
            Style::default().fg(Color::DarkGray)
        } else if is_focused {
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary)
        };
        let para = Paragraph::new(display).style(style).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if is_focused && !is_readonly {
                    theme.border_focused
                } else {
                    theme.border
                }),
        );
        f.render_widget(para, chunks[i]);

        if is_focused || is_readonly {
            let hint = Paragraph::new(Span::styled(hints[i], theme.style_idle));
            f.render_widget(hint, chunks[3]);
        }
    }

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab/↑↓", theme.style_header),
        Span::styled(" switch  ", theme.style_idle),
        Span::styled("Ctrl+V", theme.style_header),
        Span::styled(" paste  ", theme.style_idle),
        Span::styled("Enter", theme.style_header),
        Span::styled(" save  ", theme.style_idle),
        Span::styled("Esc", theme.style_header),
        Span::styled(" cancel", theme.style_idle),
    ]));
    f.render_widget(footer, chunks[4]);
    f.render_widget(block, area);
}
