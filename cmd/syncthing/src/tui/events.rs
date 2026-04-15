use std::collections::VecDeque;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use std::str::FromStr;

use syncthing_core::types::{AddressType, Device, Folder};
use syncthing_core::DeviceId;

use crate::tui::app::{App, FormState, Popup, Tab};
use crate::save_config;

pub fn handle_event(app: &mut App, event: Event) -> bool {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key(app, key),
        _ => false,
    }
}

/// 返回 true 表示应该退出 TUI
fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    // 全局快捷键
    if key.code == KeyCode::Char('q') && app.popup == Popup::None {
        return true;
    }

    if key.code == KeyCode::F(5) {
        return false; // F5 由调用方处理启动/停止 daemon
    }

    match app.popup {
        Popup::AddDevice => return handle_add_device_key(app, key),
        Popup::AddFolder => return handle_add_folder_key(app, key),
        Popup::Error(_) => {
            app.popup = Popup::None;
            return false;
        }
        Popup::None => {}
    }

    match key.code {
        KeyCode::Right | KeyCode::Tab => app.tab = app.tab.next(),
        KeyCode::Left | KeyCode::BackTab => app.tab = app.tab.prev(),
        KeyCode::Char('a') => {
            match app.tab {
                Tab::Devices => {
                    app.device_form = FormState::new(vec![String::new(), String::new(), String::new()]);
                    app.popup = Popup::AddDevice;
                }
                Tab::Folders => {
                    app.folder_form = FormState::new(vec![String::new(), String::new()]);
                    app.resize_form();
                    app.popup = Popup::AddFolder;
                }
                _ => {}
            }
        }
        KeyCode::Char('d') => {
            match app.tab {
                Tab::Devices => {
                    if !app.config.devices.is_empty() {
                        let id = app.config.devices[app.device_selected].id;
                        app.config.devices.retain(|d| d.id != id);
                        if app.device_selected >= app.config.devices.len() && app.device_selected > 0 {
                            app.device_selected -= 1;
                        }
                        save_and_log(app);
                    }
                }
                Tab::Folders => {
                    if !app.config.folders.is_empty() {
                        app.config.folders.remove(app.folder_selected);
                        if app.folder_selected >= app.config.folders.len() && app.folder_selected > 0 {
                            app.folder_selected -= 1;
                        }
                        save_and_log(app);
                    }
                }
                _ => {}
            }
        }
        KeyCode::Down => {
            match app.tab {
                Tab::Devices => {
                    if app.device_selected + 1 < app.config.devices.len() {
                        app.device_selected += 1;
                    }
                }
                Tab::Folders => {
                    if app.folder_selected + 1 < app.config.folders.len() {
                        app.folder_selected += 1;
                    }
                }
                _ => {}
            }
        }
        KeyCode::Up => {
            match app.tab {
                Tab::Devices => {
                    if app.device_selected > 0 {
                        app.device_selected -= 1;
                    }
                }
                Tab::Folders => {
                    if app.folder_selected > 0 {
                        app.folder_selected -= 1;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }

    false
}

fn handle_add_device_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => app.popup = Popup::None,
        KeyCode::Tab => {
            app.device_form.focus = (app.device_form.focus + 1) % app.device_form.fields.len();
        }
        KeyCode::BackTab => {
            if app.device_form.focus == 0 {
                app.device_form.focus = app.device_form.fields.len() - 1;
            } else {
                app.device_form.focus -= 1;
            }
        }
        KeyCode::Enter => {
            let id_str = app.device_form.fields[0].trim();
            let name = app.device_form.fields[1].trim();
            let addr = app.device_form.fields[2].trim();

            if id_str.is_empty() {
                app.popup = Popup::Error("Device ID cannot be empty".to_string());
                return false;
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
                        name: if name.is_empty() { None } else { Some(name.to_string()) },
                        addresses,
                        paused: false,
                        introducer: false,
                    });
                    app.popup = Popup::None;
                    save_and_log(app);
                }
                Err(e) => {
                    app.popup = Popup::Error(format!("Invalid Device ID: {}", e));
                }
            }
        }
        KeyCode::Char(c) => {
            if let Some(field) = app.device_form.fields.get_mut(app.device_form.focus) {
                field.push(c);
            }
        }
        KeyCode::Backspace => {
            if let Some(field) = app.device_form.fields.get_mut(app.device_form.focus) {
                field.pop();
            }
        }
        _ => {}
    }
    false
}

fn handle_add_folder_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => app.popup = Popup::None,
        KeyCode::Tab => {
            app.folder_form.focus = (app.folder_form.focus + 1) % (app.folder_form.fields.len() + 1);
        }
        KeyCode::BackTab => {
            let len = app.folder_form.fields.len() + 1;
            if app.folder_form.focus == 0 {
                app.folder_form.focus = len - 1;
            } else {
                app.folder_form.focus -= 1;
            }
        }
        KeyCode::Enter => {
            let id = app.folder_form.fields[0].trim();
            let path = app.folder_form.fields[1].trim();

            if id.is_empty() || path.is_empty() {
                app.popup = Popup::Error("Folder ID and Path cannot be empty".to_string());
                return false;
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
            app.config.folders.push(folder);
            app.popup = Popup::None;
            save_and_log(app);
        }
        KeyCode::Down => {
            if app.folder_form.focus == app.folder_form.fields.len() {
                if app.folder_device_selected + 1 < app.config.devices.len() {
                    app.folder_device_selected += 1;
                }
            }
        }
        KeyCode::Up => {
            if app.folder_form.focus == app.folder_form.fields.len() {
                if app.folder_device_selected > 0 {
                    app.folder_device_selected -= 1;
                }
            }
        }
        KeyCode::Char(' ') => {
            if app.folder_form.focus == app.folder_form.fields.len() {
                if let Some(selected) = app.folder_device_selection.get_mut(app.folder_device_selected) {
                    *selected = !*selected;
                }
            }
        }
        KeyCode::Char(c) => {
            if let Some(field) = app.folder_form.fields.get_mut(app.folder_form.focus) {
                field.push(c);
            }
        }
        KeyCode::Backspace => {
            if let Some(field) = app.folder_form.fields.get_mut(app.folder_form.focus) {
                field.pop();
            }
        }
        _ => {}
    }
    false
}

fn save_and_log(app: &mut App) {
    let path = app.config_dir.join("config.json");
    match save_config(&path, &app.config) {
        Ok(_) => app.push_log("Config saved.".to_string()),
        Err(e) => app.push_log(format!("Failed to save config: {}", e)),
    }
}
