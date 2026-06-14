use ratatui::{
    layout::{Alignment, Rect},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::{App, Popup, Tab};
use crate::tui::forms::render;

/// 明确切分终端区域，避免 `Constraint::Min(0)` 在 Windows 控制台首帧分配异常
/// 导致主内容区域与底部状态栏重叠一行。
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    if area.width < 40 || area.height < 12 {
        let msg = Paragraph::new("Terminal too small. Please resize to at least 40x12.")
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        f.render_widget(msg, area);
        return;
    }

    // 固定 header 3 行、status 1 行，main 占中间剩余全部行数。
    // 使用显式 Rect 而不是 Layout 的 Min(0)，确保首帧不会出现分配漂移。
    let header_area = Rect::new(0, 0, area.width, 3);
    let status_area = Rect::new(0, area.height - 1, area.width, 1);
    let main_area = Rect::new(0, 3, area.width, area.height - 4);

    crate::tui::widgets::header::draw(f, app, header_area);
    draw_main_content(f, app, main_area);

    // 先清空状态栏区域，再绘制，防止主内容或旧帧残留覆盖/截断状态栏。
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
