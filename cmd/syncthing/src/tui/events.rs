use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};

use std::str::FromStr;
use std::sync::Arc;

use syncthing_core::types::{AddressType, Device, Folder};
use syncthing_core::DeviceId;

use crate::save_config;
use crate::tui::app::{App, Popup, Tab};
use crate::tui::forms::{FormAction, FormState};

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
            return handle_popup_key(app, key);
        }
        Popup::Help | Popup::Error(_) => {
            app.popup = Popup::None;
            return false;
        }
        Popup::None => {}
    }

    // Tab-mode shortcuts
    handle_tab_key(app, key)
}

/// 处理弹窗内的按键（所有表单弹窗的统一入口）
fn handle_popup_key(app: &mut App, key: KeyEvent) -> bool {
    let form = match &mut app.form {
        Some(f) => f,
        None => return false,
    };

    // 文件夹表单的 device 列表特殊处理
    let is_folder = matches!(app.popup, Popup::AddFolder | Popup::EditFolder);

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
    let form = match &app.form {
        Some(f) => f,
        None => return,
    };
    let id_str = form.fields[0].value.trim();
    let name = form.fields[1].value.trim();
    let addr = form.fields[2].value.trim();

    if let Err(e) = syncthing_core::validation::validate_device_id(id_str) {
        app.popup = Popup::Error(e.to_string());
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
            app.popup = Popup::Error(format!("Invalid Device ID: {}", e));
        }
    }
}

fn submit_edit_device(app: &mut App) {
    let form = match &app.form {
        Some(f) => f,
        None => return,
    };
    let name = form.fields[1].value.trim();
    let addr = form.fields[2].value.trim();

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
        app.form = None;
        app.popup = Popup::None;
        save_and_log(app);
    }
}

fn submit_add_folder(app: &mut App) {
    let form = match &app.form {
        Some(f) => f,
        None => return,
    };
    let id = form.fields[0].value.trim();
    let path = form.fields[1].value.trim();

    if let Err(e) = syncthing_core::validation::validate_folder_id(id) {
        app.popup = Popup::Error(e.to_string());
        return;
    }
    if let Err(e) = syncthing_core::validation::validate_path(path) {
        app.popup = Popup::Error(e.to_string());
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
    app.config.folders.push(folder);
    app.form = None;
    app.popup = Popup::None;
    save_and_log(app);
}

fn submit_edit_folder(app: &mut App) {
    let form = match &app.form {
        Some(f) => f,
        None => return,
    };
    let path = form.fields[1].value.trim();

    if let Err(e) = syncthing_core::validation::validate_path(path) {
        app.popup = Popup::Error(e.to_string());
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
        app.form = None;
        app.popup = Popup::None;
        save_and_log(app);
    }
}

/// Tab 非弹窗模式下的按键处理
fn handle_tab_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Right | KeyCode::Tab => app.tab = app.tab.next(),
        KeyCode::Left | KeyCode::BackTab => app.tab = app.tab.prev(),

        KeyCode::Char('a') | KeyCode::Insert => match app.tab {
            Tab::Devices => {
                let form = FormState::new("Add Device", 72, 14)
                    .add_field("Device ID", String::new(), true, None)
                    .add_field("Name", String::new(), true, None)
                    .add_field("Address", String::new(), true, None);
                app.form = Some(form);
                app.popup = Popup::AddDevice;
            }
            Tab::Folders => {
                let form = FormState::new("Add Folder", 60, 16)
                    .add_field("Folder ID", String::new(), true, None)
                    .add_field(
                        "Path",
                        String::new(),
                        true,
                        Some("Absolute path, e.g. C:\\Users\\me\\sync"),
                    );
                app.resize_form();
                app.form = Some(form);
                app.popup = Popup::AddFolder;
            }
            _ => {}
        },

        KeyCode::Char('d') | KeyCode::Delete => match app.tab {
            Tab::Devices if !app.config.devices.is_empty() => {
                let id = app.config.devices[app.device_selected].id;
                app.config.devices.retain(|d| d.id != id);
                for folder in &mut app.config.folders {
                    folder.devices.retain(|&did| did != id);
                }
                if app.device_selected >= app.config.devices.len() && app.device_selected > 0 {
                    app.device_selected -= 1;
                }
                app.resize_form();
                save_and_log(app);
            }
            Tab::Folders if !app.config.folders.is_empty() => {
                app.config.folders.remove(app.folder_selected);
                if app.folder_selected >= app.config.folders.len() && app.folder_selected > 0 {
                    app.folder_selected -= 1;
                }
                save_and_log(app);
            }
            _ => {}
        },

        KeyCode::Down => match app.tab {
            Tab::Devices if app.device_selected + 1 < app.config.devices.len() => {
                app.device_selected += 1;
            }
            Tab::Folders if app.folder_selected + 1 < app.config.folders.len() => {
                app.folder_selected += 1;
            }
            _ => {}
        },

        KeyCode::Up => match app.tab {
            Tab::Devices if app.device_selected > 0 => {
                app.device_selected -= 1;
            }
            Tab::Folders if app.folder_selected > 0 => {
                app.folder_selected -= 1;
            }
            _ => {}
        },

        KeyCode::Enter | KeyCode::Char('e') => match app.tab {
            Tab::Devices if !app.config.devices.is_empty() => {
                if let Some(device) = app.config.devices.get(app.device_selected) {
                    let addr = device
                        .addresses
                        .first()
                        .map(|a| a.to_string())
                        .unwrap_or_default();
                    let form = FormState::new("Edit Device", 72, 14)
                        .add_field("Device ID", device.id.to_string(), false, None)
                        .add_field("Name", device.name.clone().unwrap_or_default(), true, None)
                        .add_field("Address", addr, true, None);
                    app.form = Some(form);
                    app.popup = Popup::EditDevice;
                }
            }
            Tab::Folders if !app.config.folders.is_empty() => {
                if let Some(folder) = app.config.folders.get(app.folder_selected) {
                    let form = FormState::new("Edit Folder", 60, 16)
                        .add_field(
                            "Folder ID",
                            folder.id.clone(),
                            false,
                            Some("Folder ID cannot be changed"),
                        )
                        .add_field(
                            "Path",
                            folder.path.clone(),
                            true,
                            Some("Absolute path, e.g. C:\\Users\\me\\sync"),
                        );
                    app.folder_device_selected = 0;
                    app.folder_device_selection = app
                        .config
                        .devices
                        .iter()
                        .map(|d| folder.devices.contains(&d.id))
                        .collect();
                    app.form = Some(form);
                    app.popup = Popup::EditFolder;
                }
            }
            _ => {}
        },

        KeyCode::Char('i') if app.tab == Tab::Folders => {
            if let Some(folder) = app.config.folders.get(app.folder_selected) {
                let stignore = std::path::Path::new(&folder.path).join(".stignore");
                if !stignore.exists() {
                    let _ = std::fs::write(&stignore, "# .stignore — syncthing-rust\n");
                }
                let path_str = stignore.to_string_lossy().to_string();
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("notepad.exe")
                        .arg(&path_str)
                        .spawn();
                }
                #[cfg(not(windows))]
                {
                    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
                    let _ = std::process::Command::new(editor).arg(&path_str).spawn();
                }
                if let Some(ref service) = app.sync_service {
                    let service = Arc::clone(service);
                    let fid = folder.id.clone();
                    tokio::spawn(async move {
                        let _ = service.scan_folder(&fid).await;
                    });
                }
            }
        }

        _ => {}
    }

    false
}

fn save_and_log(app: &mut App) {
    let path = app.config_dir.join("config.json");
    match save_config(&path, &app.config) {
        Ok(_) => {
            app.push_log("Config saved.".to_string());
            if let Some(ref service) = app.sync_service {
                let service = Arc::clone(service);
                let config = app.config.clone();
                tokio::spawn(async move {
                    if let Err(e) = service.update_config(config).await {
                        tracing::warn!("Failed to update sync service config: {}", e);
                    }
                });
            }
        }
        Err(e) => app.push_log(format!("Failed to save config: {}", e)),
    }
}
