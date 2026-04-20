use ratatui::{
    text::Line,
    widgets::{Block, Borders, Tabs},
    Frame,
};

use crate::tui::app::{App, Tab};

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let theme = &app.theme;
    let titles: Vec<Line> = [Tab::Overview, Tab::Devices, Tab::Folders, Tab::Logs]
        .iter()
        .map(|t| Line::from(t.title()))
        .collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border)
                .title(Span::styled(" syncthing-rust ", theme.style_header)),
        )
        .highlight_style(theme.style_header)
        .select(app.tab as usize);

    f.render_widget(tabs, area);
}

use ratatui::text::Span;
