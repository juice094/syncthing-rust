use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::{App, Popup, Tab};
use crate::tui::forms::render;

/// Draws the full TUI frame.
///
/// See "docs/design/ADR-20261105-tui-first-frame-rendering.md" for why the
/// status bar is a fixed one-row layout chunk (a `Constraint::Min(0)` first
/// frame on Windows consoles used to overlap it with the main content).
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
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let header_area = chunks[0];
    let main_area = chunks[1];
    let status_area = chunks[2];

    crate::tui::widgets::header::draw(f, app, header_area);
    draw_main_content(f, app, main_area);

    f.render_widget(Clear, status_area);
    crate::tui::widgets::status_bar::draw(f, app, status_area);

    let theme = &app.theme;
    match &app.popup {
        Popup::AddDevice | Popup::EditDevice => {
            if let Some(ref form) = app.form {
                render::draw_device_form(f, form, theme);
            }
        }
        Popup::AddFolder | Popup::EditFolder => {
            if let Some(ref form) = app.form {
                render::draw_folder_form(
                    f,
                    form,
                    theme,
                    &app.config.devices,
                    &app.folder_device_selection,
                    app.folder_device_selected,
                );
            }
        }
        Popup::Help => crate::tui::popups::help::draw(f, theme),
        Popup::Error(ref msg) => crate::tui::popups::error::draw(f, msg, theme),
        Popup::Search { ref query } => draw_search_popup(f, query, theme),
        Popup::Filter { ref query } => draw_filter_popup(f, query, theme),
        Popup::Confirm {
            ref title,
            ref message,
        } => {
            crate::tui::popups::confirm::draw(f, title, message, theme);
        }
        Popup::None => {}
    }
}

fn draw_search_popup(f: &mut Frame, query: &str, theme: &crate::tui::theme::Theme) {
    let area = f.area();
    let width = 60.min(area.width.saturating_sub(4));
    let height = 3.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + area.height.saturating_sub(height).saturating_sub(1);
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let text = format!("Search logs: {}_", query);
    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_focused)
            .title(" Search ")
            .title_style(Style::default().fg(theme.text_primary)),
    );
    f.render_widget(paragraph, popup_area);
}

fn draw_filter_popup(f: &mut Frame, query: &str, theme: &crate::tui::theme::Theme) {
    let area = f.area();
    let width = 60.min(area.width.saturating_sub(4));
    let height = 3.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + area.height.saturating_sub(height).saturating_sub(1);
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let text = format!("Filter: {}_", query);
    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border_focused)
            .title(" Filter ")
            .title_style(Style::default().fg(theme.text_primary)),
    );
    f.render_widget(paragraph, popup_area);
}

fn draw_main_content(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    match app.tab {
        Tab::Overview => crate::tui::views::overview::draw(f, app, area),
        Tab::Devices => crate::tui::views::devices::draw(f, app, area),
        Tab::Folders => crate::tui::views::folders::draw(f, app, area),
        Tab::Logs => crate::tui::views::logs::draw(f, app, area),
    }
}
