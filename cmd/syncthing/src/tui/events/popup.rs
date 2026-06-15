use std::str::FromStr;

use crossterm::event::{KeyCode, KeyEvent};

use syncthing_core::types::{AddressType, Device, Folder};
use syncthing_core::DeviceId;

use crate::tui::app::{App, Popup};
use crate::tui::forms::FormAction;

use super::tab::save_and_log;

/// 处理弹窗内的按键（所有表单弹窗的统一入口）
pub fn handle_popup_key(app: &mut App, key: KeyEvent) -> bool {
    let form = match &mut app.form {
        Some(f) => f,
        None => return false,
    };

    // 文件夹表单的 device 列表特殊处理
    let is_folder = matches!(app.popup, Popup::AddFolder | Popup::EditFolder);

    // 从文本字段 Tab/Down 进入 Share with 列表；BackTab/Up 从首个字段回绕进入列表
    if is_folder && !form.is_on_list() {
        let last_field = form.field_count().saturating_sub(1);
        match key.code {
            KeyCode::Tab | KeyCode::Down if form.focus == last_field => {
                form.focus = form.fields.len();
                return false;
            }
            KeyCode::BackTab | KeyCode::Up if form.focus == 0 => {
                form.focus = form.fields.len();
                return false;
            }
            _ => {}
        }
    }

    if is_folder && form.is_on_list() {
        match key.code {
            KeyCode::Down => {
                if app.folder_device_selected + 1 < app.config.devices.len() {
                    app.folder_device_selected += 1;
                }
                return false;
            }
            KeyCode::Up => {
                if app.folder_device_selected > 0 {
                    app.folder_device_selected -= 1;
                }
                return false;
            }
            KeyCode::Char(' ') => {
                if let Some(sel) = app
                    .folder_device_selection
                    .get_mut(app.folder_device_selected)
                {
                    *sel = !*sel;
                }
                return false;
            }
            KeyCode::Tab => {
                form.focus = 0;
                return false;
            }
            KeyCode::BackTab => {
                form.focus = form.field_count().saturating_sub(1);
                return false;
            }
            _ => {}
        }
        // Other keys on device list: only Space, Tab, Up, Down matter
        if !matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
            return false;
        }
    }

    let action = crate::tui::forms::handler::handle_form_key(form, key);

    match action {
        FormAction::Cancel => {
            app.form = None;
            app.popup = Popup::None;
        }
        FormAction::Submit => {
            submit_form(app);
        }
        FormAction::Continue => {}
    }

    false
}

/// 提交当前表单（验证 + 保存）
fn submit_form(app: &mut App) {
    match app.popup {
        Popup::AddDevice => submit_add_device(app),
        Popup::EditDevice => submit_edit_device(app),
        Popup::AddFolder => submit_add_folder(app),
        Popup::EditFolder => submit_edit_folder(app),
        _ => {}
    }
}

fn submit_add_device(app: &mut App) {
    let form = match app.form.as_mut() {
        Some(f) => f,
        None => return,
    };
    let id_str = form.value("device_id").unwrap_or_default().trim();
    let name = form.value("device_name").unwrap_or_default().trim();
    let addr = form.value("address").unwrap_or_default().trim();

    if let Err(e) = syncthing_core::validation::validate_device_id(id_str) {
        form.set_error(e.to_string());
        return;
    }

    match DeviceId::from_str(id_str) {
        Ok(id) => {
            let addresses = if addr.is_empty() {
                vec![AddressType::Dynamic]
            } else {
                vec![AddressType::Tcp(addr.to_string())]
            };
            app.config.devices.push(Device {
                id,
                name: if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                },
                addresses,
                paused: false,
                introducer: false,
            });
            app.resize_form();
            app.form = None;
            app.popup = Popup::None;
            save_and_log(app);
        }
        Err(e) => {
            form.set_error(format!("Invalid Device ID: {}", e));
        }
    }
}

fn submit_edit_device(app: &mut App) {
    let form = match app.form.as_mut() {
        Some(f) => f,
        None => return,
    };
    let name = form.value("device_name").unwrap_or_default().trim();
    let addr = form.value("address").unwrap_or_default().trim();

    let selected = app.device_selected;
    if let Some(device) = app.config.devices.get_mut(selected) {
        let addresses = if addr.is_empty() {
            vec![AddressType::Dynamic]
        } else {
            vec![AddressType::Tcp(addr.to_string())]
        };
        device.name = if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        };
        device.addresses = addresses;
        form.clear_error();
        app.form = None;
        app.popup = Popup::None;
        save_and_log(app);
    }
}

fn submit_add_folder(app: &mut App) {
    let form = match app.form.as_mut() {
        Some(f) => f,
        None => return,
    };
    let id = form.value("folder_id").unwrap_or_default().trim();
    let path = form.value("path").unwrap_or_default().trim();

    if let Err(e) = syncthing_core::validation::validate_folder_id(id) {
        form.set_error(e.to_string());
        return;
    }
    if let Err(e) = syncthing_core::validation::validate_path(path) {
        form.set_error(e.to_string());
        return;
    }

    let mut folder = Folder::new(id, path);
    let local_id = app.config.local_device_id.unwrap_or_default();
    folder.devices.push(local_id);
    for (i, selected) in app.folder_device_selection.iter().enumerate() {
        if *selected {
            if let Some(device) = app.config.devices.get(i) {
                folder.devices.push(device.id);
            }
        }
    }
    form.clear_error();
    app.form = None;
    app.popup = Popup::None;
    save_and_log(app);
}

fn submit_edit_folder(app: &mut App) {
    let form = match app.form.as_mut() {
        Some(f) => f,
        None => return,
    };
    let path = form.value("path").unwrap_or_default().trim();

    if let Err(e) = syncthing_core::validation::validate_path(path) {
        form.set_error(e.to_string());
        return;
    }

    let selected = app.folder_selected;
    if let Some(folder) = app.config.folders.get_mut(selected) {
        folder.path = path.to_string();
        let local_id = app.config.local_device_id.unwrap_or_default();
        folder.devices.clear();
        folder.devices.push(local_id);
        for (i, selected) in app.folder_device_selection.iter().enumerate() {
            if *selected {
                if let Some(device) = app.config.devices.get(i) {
                    folder.devices.push(device.id);
                }
            }
        }
        form.clear_error();
        app.form = None;
        app.popup = Popup::None;
        save_and_log(app);
    }
}
