use crossterm::event::{KeyCode, KeyEvent};

use crate::tui::app::{App, Popup};

/// 处理确认对话框按键
pub fn handle_confirm_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(cb) = app.confirm_callback.take() {
                cb(app);
            }
            app.popup = Popup::None;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.confirm_callback = None;
            app.popup = Popup::None;
        }
        _ => {}
    }
    false
}
