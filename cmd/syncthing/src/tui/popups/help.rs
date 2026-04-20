use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::theme::Theme;
use crate::tui::widgets::centered_rect;

pub fn draw(f: &mut Frame, theme: &Theme) {
    let area = centered_rect(70, 80, f.area());

    // Dim background
    let dim = ratatui::widgets::Block::default().style(Style::default().bg(ratatui::style::Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());

    f.render_widget(Clear, area);

    let shortcuts = vec![
        ("Global", vec![
            ("F5", "Start / Stop daemon"),
            ("Tab / ← →", "Switch tab"),
            ("q", "Quit TUI"),
            ("?", "Show this help"),
        ]),
        ("Navigation", vec![
            ("↑ ↓", "Select item in list"),
            ("Enter", "Open detail view (when available)"),
        ]),
        ("Devices Tab", vec![
            ("a", "Add new device"),
            ("d", "Delete selected device"),
        ]),
        ("Folders Tab", vec![
            ("a", "Add new folder"),
            ("d", "Delete selected folder"),
        ]),
        ("Forms (Add Device / Folder)", vec![
            ("Tab", "Next field"),
            ("Shift+Tab", "Previous field"),
            ("↑ ↓", "Navigate device list (folder form)"),
            ("Space", "Toggle device checkbox"),
            ("Enter", "Save"),
            ("Esc", "Cancel"),
        ]),
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled("Keyboard Shortcuts", theme.style_header.add_modifier(Modifier::BOLD))),
        Line::raw(""),
    ];

    for (section, keys) in shortcuts {
        lines.push(Line::from(Span::styled(
            format!(" {} ", section),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        for (key, desc) in keys {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:15}", key), theme.style_header),
                Span::styled(desc.to_string(), Style::default().fg(theme.text_primary)),
            ]));
        }
        lines.push(Line::raw(""));
    }

    lines.push(Line::from(Span::styled(
        "Press Esc or ? to close",
        theme.style_idle,
    )));

    let text = Text::from(lines);
    let para = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border_focused)
                .title(Span::styled(" Help ", theme.style_header)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}
