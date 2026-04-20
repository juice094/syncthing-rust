use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Tabs},
    Frame,
};

use crate::tui::app::{App, Tab};

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let titles: Vec<Line> = [Tab::Overview, Tab::Devices, Tab::Folders, Tab::Logs]
        .iter()
        .map(|t| Line::from(t.title()))
        .collect();

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("syncthing-rust"))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .select(app.tab as usize);

    f.render_widget(tabs, area);
}
