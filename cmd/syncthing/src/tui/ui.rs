use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::tui::app::{App, Popup, Tab};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    if area.width < 40 || area.height < 12 {
        let msg = Paragraph::new("Terminal too small. Please resize to at least 40x12.")
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        f.render_widget(msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    crate::tui::widgets::header::draw(f, app, chunks[0]);
    draw_main_content(f, app, chunks[1]);
    crate::tui::widgets::status_bar::draw(f, app, chunks[2]);

    let theme = &app.theme;
    match app.popup {
        Popup::AddDevice => crate::tui::popups::add_device::draw(f, app, theme),
        Popup::AddFolder => crate::tui::popups::add_folder::draw(f, app, theme),
        Popup::Error(ref msg) => crate::tui::popups::error::draw(f, msg, theme),
        Popup::None => {}
    }
}

fn draw_main_content(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    match app.tab {
        Tab::Overview => crate::tui::views::overview::draw(f, app, area),
        Tab::Devices => crate::tui::views::devices::draw(f, app, area),
        Tab::Folders => crate::tui::views::folders::draw(f, app, area),
        Tab::Logs => crate::tui::views::logs::draw(f, app, area),
    }
}
