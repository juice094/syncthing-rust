use crossterm::event::{KeyCode, KeyEvent};

use crate::tui::app::App;

/// 处理 Logs 页快捷键；返回 true 表示已消费该按键。
pub fn handle_logs_key(app: &mut App, key: KeyEvent) -> bool {
    let page_size = log_page_size();
    let max_offset = app.max_log_scroll_offset();

    match key.code {
        KeyCode::Char('/') => {
            let initial = app.log_search.clone().unwrap_or_default();
            app.popup = crate::tui::app::Popup::Search { query: initial };
            return true;
        }

        KeyCode::Char('j') | KeyCode::Down => {
            app.log_scroll_offset = app.log_scroll_offset.saturating_sub(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.log_scroll_offset = (app.log_scroll_offset + 1).min(max_offset);
        }
        KeyCode::PageUp => {
            app.log_scroll_offset = (app.log_scroll_offset + page_size).min(max_offset);
        }
        KeyCode::PageDown => {
            app.log_scroll_offset = app.log_scroll_offset.saturating_sub(page_size);
        }
        KeyCode::Home => {
            app.log_scroll_offset = max_offset;
        }
        KeyCode::End => {
            app.log_scroll_offset = 0;
        }

        KeyCode::Char('n') => {
            if !app.log_search_matches.is_empty() {
                app.log_search_selected =
                    (app.log_search_selected + 1).min(app.log_search_matches.len() - 1);
                app.log_scroll_offset = app.log_search_matches.len() - 1 - app.log_search_selected;
            }
        }
        KeyCode::Char('N') => {
            if !app.log_search_matches.is_empty() {
                app.log_search_selected = app.log_search_selected.saturating_sub(1);
                app.log_scroll_offset = app.log_search_matches.len() - 1 - app.log_search_selected;
            }
        }

        KeyCode::Esc => {
            app.log_search = None;
            app.log_search_matches.clear();
            app.log_search_selected = 0;
            app.log_scroll_offset = 0;
        }

        _ => return false,
    }

    true
}

/// 估算每屏可显示的日志行数。
fn log_page_size() -> usize {
    crossterm::terminal::size()
        .map(|(_, h)| h.saturating_sub(6).max(1) as usize)
        .unwrap_or(10)
}
