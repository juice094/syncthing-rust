use ratatui::{
    widgets::{Block, Borders, List, ListDirection},
    Frame,
};

use crate::tui::app::App;
use crate::tui::widgets::log_line::colored_log_line;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    // 使用 List 而非 Paragraph + Wrap，避免长日志自动换行后垂直溢出到状态栏。
    // 每行日志作为一个 ListItem，超出宽度时自动截断。
    let items: Vec<ratatui::widgets::ListItem> = app
        .log_lines
        .iter()
        .rev()
        .map(|line| ratatui::widgets::ListItem::new(colored_log_line(line.as_str(), &app.theme)))
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Logs"))
        .direction(ListDirection::BottomToTop);
    f.render_widget(list, area);
}
