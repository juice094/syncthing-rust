use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::app::App;
use crate::tui::theme::Theme;
use crate::tui::widgets::centered_rect;

pub fn draw(f: &mut Frame, app: &App, theme: &Theme) {
    let area = centered_rect(60, 40, f.area());

    // Dim background
    let dim = ratatui::widgets::Block::default().style(Style::default().bg(Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());

    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused)
        .title(Span::styled(" Add Device ", theme.style_header));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);

    let labels = [("Device ID", 0), ("Name", 1), ("Address", 2)];
    let hints = [
        "The long ID string from the remote device",
        "Optional display name",
        "e.g. 127.0.0.1:22001 (leave empty for dynamic)",
    ];

    for (i, (label, field_idx)) in labels.iter().enumerate() {
        let text = format!(
            "{}: {}",
            label,
            app.device_form.fields.get(*field_idx).map(|s| s.as_str()).unwrap_or("")
        );
        let is_focused = app.device_form.focus == *field_idx;
        let style = if is_focused {
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_secondary)
        };
        let para = Paragraph::new(text)
            .style(style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(if is_focused { theme.border_focused } else { theme.border }),
            );
        f.render_widget(para, chunks[i]);

        // Hint text below field
        if is_focused {
            let hint = Paragraph::new(Span::styled(hints[i], theme.style_idle));
            f.render_widget(hint, chunks[3]);
        }
    }

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab", theme.style_header),
        Span::styled(" next field  ", theme.style_idle),
        Span::styled("Enter", theme.style_header),
        Span::styled(" save  ", theme.style_idle),
        Span::styled("Esc", theme.style_header),
        Span::styled(" cancel", theme.style_idle),
    ]));
    f.render_widget(footer, chunks[4]);

    f.render_widget(block, area);
}
