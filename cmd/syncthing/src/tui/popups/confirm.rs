use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::constants;
use crate::tui::theme::Theme;
use crate::tui::widgets::centered_popup;

pub fn draw(f: &mut Frame, title: &str, message: &str, theme: &Theme) {
    let area = centered_popup(
        constants::CONFIRM_POPUP_W,
        constants::CONFIRM_POPUP_H,
        f.area(),
    );

    // Dim the background
    let dim = ratatui::widgets::Block::default().style(Style::default().bg(Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());

    f.render_widget(Clear, area);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(message, theme.style_header)),
        Line::raw(""),
    ];
    lines.push(Line::from(Span::styled(
        "Press y to confirm, n or Esc to cancel",
        theme.style_idle,
    )));

    let para = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.style_popup_border)
                .title(format!(" {title} ")),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}
