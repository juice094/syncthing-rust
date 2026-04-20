use ratatui::{
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::App;
use crate::tui::widgets::log_line::colored_logs;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let lines: Vec<String> = app.log_lines.iter().cloned().collect();
    let colored = colored_logs(&lines, &app.theme);

    let para = Paragraph::new(colored)
        .block(Block::default().borders(Borders::ALL).title("Logs"))
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}
