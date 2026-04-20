use ratatui::{
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::widgets::centered_rect;

pub fn draw(f: &mut Frame, msg: &str) {
    let area = centered_rect(50, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(msg)
        .block(Block::default().borders(Borders::ALL).title("Error (Esc to close)"))
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}
