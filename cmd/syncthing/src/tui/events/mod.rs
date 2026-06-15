use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};

use crate::tui::app::{App, Popup, Tab};

mod confirm;
mod logs;
mod popup;
mod search;
mod tab;

/// 处理输入事件。返回 `true` 表示应当退出 TUI。
pub fn handle_event(app: &mut App, event: &Event) -> bool {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key(app, *key),
        _ => false,
    }
}

/// 返回 true 表示应该退出 TUI
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    // 全局快捷键
    if key.code == KeyCode::Char('q') && app.popup == Popup::None {
        return true;
    }

    // F5 由 run_app 事件循环直接处理（需要 daemon 生命周期句柄），此处不拦截。
    if key.code == KeyCode::F(5) {
        return false;
    }

    if key.code == KeyCode::Char('?') && app.popup == Popup::None {
        app.popup = Popup::Help;
        return false;
    }

    // 日志级别过滤
    if key.code == KeyCode::Char('l') && app.popup == Popup::None {
        app.log_filter_level = match app.log_filter_level {
            tracing::Level::ERROR => tracing::Level::WARN,
            tracing::Level::WARN => tracing::Level::INFO,
            tracing::Level::INFO => tracing::Level::DEBUG,
            tracing::Level::DEBUG => tracing::Level::TRACE,
            tracing::Level::TRACE => tracing::Level::ERROR,
        };
        app.push_log(format!(
            "Log filter changed to {}",
            app.log_filter_level.as_str().to_ascii_uppercase()
        ));
        return false;
    }

    // Popup dispatch
    match app.popup {
        Popup::AddDevice | Popup::EditDevice | Popup::AddFolder | Popup::EditFolder => {
            return popup::handle_popup_key(app, key);
        }
        Popup::Help | Popup::Error(_) => {
            app.popup = Popup::None;
            return false;
        }
        Popup::Search { .. } => {
            return search::handle_search_popup_key(app, key);
        }
        Popup::Filter { .. } => {
            return search::handle_filter_popup_key(app, key);
        }
        Popup::Confirm { .. } => {
            return confirm::handle_confirm_key(app, key);
        }
        Popup::None => {}
    }

    // Logs tab shortcuts（优先于通用 Tab 导航）
    if app.tab == Tab::Logs && logs::handle_logs_key(app, key) {
        return false;
    }

    // Tab-mode shortcuts
    tab::handle_tab_key(app, key)
}
