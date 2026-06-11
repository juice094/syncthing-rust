use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::constants;
use crate::tui::theme::Theme;
use crate::tui::widgets::centered_popup;

pub fn draw(f: &mut Frame, msg: &str, theme: &Theme) {
    let area = centered_popup(constants::ERROR_POPUP_W, constants::ERROR_POPUP_H, f.area());

    // Dim the background
    let dim = ratatui::widgets::Block::default().style(Style::default().bg(Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());

    f.render_widget(Clear, area);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled("⚠ Error", theme.style_error)),
        Line::raw(""),
    ];
    for line in msg.lines() {
        lines.push(Line::raw(line));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Press Esc to close",
        theme.style_idle,
    )));

    let para = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.style_error)
                .title(" Error "),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}
