use crossterm::event::{KeyCode, KeyEvent};

use crate::tui::app::{App, Popup};

/// Handles key presses in the Devices / Folders filter popup.
pub fn handle_filter_popup_key(app: &mut App, key: KeyEvent) -> bool {
    let mut query = match std::mem::replace(&mut app.popup, Popup::None) {
        Popup::Filter { query } => query,
        _ => return false,
    };

    match key.code {
        KeyCode::Enter => {
            app.list_filter = Some(query);
            app.list_filter_selected = 0;
            app.recompute_device_filter_matches();
            app.recompute_folder_filter_matches();
        }
        KeyCode::Esc => {}
        KeyCode::Backspace => {
            query.pop();
            app.popup = Popup::Filter { query };
        }
        KeyCode::Char(c) => {
            query.push(c);
            app.popup = Popup::Filter { query };
        }
        _ => {
            app.popup = Popup::Filter { query };
        }
    }
    false
}

/// Handles key presses in the log search popup.
pub fn handle_search_popup_key(app: &mut App, key: KeyEvent) -> bool {
    let mut query = match std::mem::replace(&mut app.popup, Popup::None) {
        Popup::Search { query } => query,
        _ => return false,
    };

    match key.code {
        KeyCode::Enter => {
            app.log_search = Some(query);
            app.recompute_log_search_matches();
            app.log_search_selected = app.log_search_matches.len().saturating_sub(1);
            app.log_scroll_offset = 0;
        }
        KeyCode::Esc => {}
        KeyCode::Backspace => {
            query.pop();
            app.popup = Popup::Search { query };
        }
        KeyCode::Char(c) => {
            query.push(c);
            app.popup = Popup::Search { query };
        }
        _ => {
            app.popup = Popup::Search { query };
        }
    }
    false
}
