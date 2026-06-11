//! 统一表单按键处理 — 替代 4 个 copy-paste 函数。
//!
//! `handle_form_key` 处理文本字段的通用操作（输入、删除、粘贴、焦点移动、提交/取消）。
//! device 列表的 Space/Up/Down 由调用方在调用前自行处理。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::FormAction;
use super::FormState;

/// 处理表单内的按键。返回 `FormAction` 指示调用方应执行的操作。
pub fn handle_form_key(form: &mut FormState, key: KeyEvent) -> FormAction {
    match key.code {
        KeyCode::Esc => {
            return FormAction::Cancel;
        }

        KeyCode::Tab if !form.is_on_list() => {
            form.focus_next(true);
        }
        KeyCode::Down if !form.is_on_list() => {
            form.focus_next(true);
        }

        KeyCode::BackTab if !form.is_on_list() => {
            form.focus_prev(true);
        }
        KeyCode::Up if !form.is_on_list() => {
            form.focus_prev(true);
        }

        KeyCode::Enter => {
            return FormAction::Submit;
        }

        KeyCode::Char('v') | KeyCode::Char('V')
            if key.modifiers.contains(KeyModifiers::CONTROL) && !form.is_on_list() =>
        {
            paste_into(form);
        }

        KeyCode::Insert if key.modifiers.contains(KeyModifiers::SHIFT) && !form.is_on_list() => {
            paste_into(form);
        }

        KeyCode::Char(c) if !form.is_on_list() => {
            form.append_char(c);
        }

        KeyCode::Backspace if !form.is_on_list() => {
            form.backspace();
        }

        _ => {}
    }

    FormAction::Continue
}

fn paste_into(form: &mut FormState) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        if let Ok(text) = clipboard.get_text() {
            if let Some(field) = form.fields.get_mut(form.focus) {
                if field.editable {
                    field.value.push_str(&text);
                }
            }
        }
    }
}
