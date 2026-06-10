use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use std::str::FromStr;
use std::sync::Arc;

use syncthing_core::types::{AddressType, Device, Folder};
use syncthing_core::DeviceId;

use crate::save_config;
use crate::tui::app::{App, FormState, Popup, Tab};

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
        return false; // F5 由调用方处理启动/停止 daemon
    }

    if key.code == KeyCode::Char('?') && app.popup == Popup::None {
        app.popup = Popup::Help;
        return false;
    }

    // P0: 日志级别过滤 — l 键循环切换 Error → Warn → Info → Debug → Trace
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

    match app.popup {
        Popup::AddDevice => return handle_add_device_key(app, key),
        Popup::AddFolder => return handle_add_folder_key(app, key),
        Popup::EditDevice => return handle_edit_device_key(app, key),
        Popup::EditFolder => return handle_edit_folder_key(app, key),
        Popup::Help => {
            app.popup = Popup::None;
            return false;
        }
        Popup::Error(_) => {
            app.popup = Popup::None;
            return false;
        }
        Popup::None => {}
    }

    match key.code {
        KeyCode::Right | KeyCode::Tab => app.tab = app.tab.next(),
        KeyCode::Left | KeyCode::BackTab => app.tab = app.tab.prev(),
        KeyCode::Char('a') | KeyCode::Insert => match app.tab {
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
        },
        KeyCode::Char('d') | KeyCode::Delete => match app.tab {
            Tab::Devices if !app.config.devices.is_empty() => {
                let id = app.config.devices[app.device_selected].id;
                app.config.devices.retain(|d| d.id != id);
                // 从所有 folder 的 devices 列表中移除该设备（清理无效的共享关系）
                for folder in &mut app.config.folders {
                    folder.devices.retain(|&did| did != id);
                }
                if app.device_selected >= app.config.devices.len() && app.device_selected > 0 {
                    app.device_selected -= 1;
                }
                // 同步 folder_device_selection 长度
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
        // Enter / e: edit selected device / folder
        KeyCode::Enter | KeyCode::Char('e') => match app.tab {
            Tab::Devices if !app.config.devices.is_empty() => {
                if let Some(device) = app.config.devices.get(app.device_selected) {
                    let addr = device
                        .addresses
                        .first()
                        .map(|a| a.to_string())
                        .unwrap_or_default();
                    app.device_form = FormState::new(vec![
                        device.id.to_string(),
                        device.name.clone().unwrap_or_default(),
                        addr,
                    ]);
                    app.popup = Popup::EditDevice;
                }
            }
            Tab::Folders if !app.config.folders.is_empty() => {
                if let Some(folder) = app.config.folders.get(app.folder_selected) {
                    app.folder_form = FormState::new(vec![folder.id.clone(), folder.path.clone()]);
                    app.folder_device_selected = 0;
                    app.folder_device_selection = app
                        .config
                        .devices
                        .iter()
                        .map(|d| folder.devices.contains(&d.id))
                        .collect();
                    app.popup = Popup::EditFolder;
                }
            }
            _ => {}
        },
        // i: open .stignore editor
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

/// 尝试从系统剪贴板粘贴文本到指定表单字段
fn try_paste_into(fields: &mut [String], focus: usize) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        if let Ok(text) = clipboard.get_text() {
            if let Some(field) = fields.get_mut(focus) {
                field.push_str(&text);
            }
        }
    }
}

fn handle_add_device_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => app.popup = Popup::None,
        KeyCode::Tab | KeyCode::Down => {
            app.device_form.focus = (app.device_form.focus + 1) % app.device_form.fields.len();
        }
        KeyCode::BackTab | KeyCode::Up => {
            if app.device_form.focus == 0 {
                app.device_form.focus = app.device_form.fields.len() - 1;
            } else {
                app.device_form.focus -= 1;
            }
        }
        KeyCode::Char('v') | KeyCode::Char('V')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            try_paste_into(&mut app.device_form.fields, app.device_form.focus);
        }
        KeyCode::Insert if key.modifiers.contains(KeyModifiers::SHIFT) => {
            try_paste_into(&mut app.device_form.fields, app.device_form.focus);
        }
        KeyCode::Enter => {
            let id_str = app.device_form.fields[0].trim();
            let name = app.device_form.fields[1].trim();
            let addr = app.device_form.fields[2].trim();

            if let Err(e) = syncthing_core::validation::validate_device_id(id_str) {
                app.popup = Popup::Error(e.to_string());
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
                        name: if name.is_empty() {
                            None
                        } else {
                            Some(name.to_string())
                        },
                        addresses,
                        paused: false,
                        introducer: false,
                    });
                    // 同步 folder_device_selection 长度，确保新设备可被添加文件夹时选中
                    app.resize_form();
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
    let max_focus = app.folder_form.fields.len();
    let device_list_focus = max_focus;
    match key.code {
        KeyCode::Esc => app.popup = Popup::None,
        KeyCode::Char('v') | KeyCode::Char('V')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            try_paste_into(&mut app.folder_form.fields, app.folder_form.focus);
        }
        KeyCode::Insert if key.modifiers.contains(KeyModifiers::SHIFT) => {
            try_paste_into(&mut app.folder_form.fields, app.folder_form.focus);
        }
        KeyCode::Enter => {
            let id = app.folder_form.fields[0].trim();
            let path = app.folder_form.fields[1].trim();

            if let Err(e) = syncthing_core::validation::validate_folder_id(id) {
                app.popup = Popup::Error(e.to_string());
                return false;
            }
            if let Err(e) = syncthing_core::validation::validate_path(path) {
                app.popup = Popup::Error(e.to_string());
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
            if app.folder_form.focus == device_list_focus
                && app.folder_device_selected + 1 < app.config.devices.len()
            {
                app.folder_device_selected += 1;
            } else {
                app.folder_form.focus = (app.folder_form.focus + 1) % (max_focus + 1);
            }
        }
        KeyCode::Up => {
            if app.folder_form.focus == device_list_focus && app.folder_device_selected > 0 {
                app.folder_device_selected -= 1;
            } else if app.folder_form.focus == 0 {
                app.folder_form.focus = max_focus;
            } else {
                app.folder_form.focus -= 1;
            }
        }
        KeyCode::Tab => {
            app.folder_form.focus = (app.folder_form.focus + 1) % (max_focus + 1);
        }
        KeyCode::BackTab => {
            if app.folder_form.focus == 0 {
                app.folder_form.focus = max_focus;
            } else {
                app.folder_form.focus -= 1;
            }
        }
        KeyCode::Char(' ') if app.folder_form.focus == device_list_focus => {
            if let Some(selected) = app
                .folder_device_selection
                .get_mut(app.folder_device_selected)
            {
                *selected = !*selected;
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

fn handle_edit_device_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => app.popup = Popup::None,
        KeyCode::Tab | KeyCode::Down => {
            // Skip field 0 (read-only Device ID)
            app.device_form.focus =
                ((app.device_form.focus + 1).max(1)) % app.device_form.fields.len();
            if app.device_form.focus == 0 {
                app.device_form.focus = 1;
            }
        }
        KeyCode::BackTab | KeyCode::Up => {
            if app.device_form.focus <= 1 {
                app.device_form.focus = app.device_form.fields.len() - 1;
            } else {
                app.device_form.focus -= 1;
            }
        }
        KeyCode::Char('v') | KeyCode::Char('V')
            if key.modifiers.contains(KeyModifiers::CONTROL) && app.device_form.focus != 0 =>
        {
            try_paste_into(&mut app.device_form.fields, app.device_form.focus);
        }
        KeyCode::Insert
            if key.modifiers.contains(KeyModifiers::SHIFT) && app.device_form.focus != 0 =>
        {
            try_paste_into(&mut app.device_form.fields, app.device_form.focus);
        }
        KeyCode::Enter => {
            let name = app.device_form.fields[1].trim();
            let addr = app.device_form.fields[2].trim();

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
                app.popup = Popup::None;
                save_and_log(app);
            }
        }
        KeyCode::Char(c) if app.device_form.focus != 0 => {
            if let Some(field) = app.device_form.fields.get_mut(app.device_form.focus) {
                field.push(c);
            }
        }
        KeyCode::Backspace if app.device_form.focus != 0 => {
            if let Some(field) = app.device_form.fields.get_mut(app.device_form.focus) {
                field.pop();
            }
        }
        _ => {}
    }
    false
}

fn handle_edit_folder_key(app: &mut App, key: KeyEvent) -> bool {
    let max_focus = app.folder_form.fields.len();
    let device_list_focus = max_focus;
    match key.code {
        KeyCode::Esc => app.popup = Popup::None,
        KeyCode::Char('v') | KeyCode::Char('V')
            if key.modifiers.contains(KeyModifiers::CONTROL) && app.folder_form.focus != 0 =>
        {
            try_paste_into(&mut app.folder_form.fields, app.folder_form.focus);
        }
        KeyCode::Insert
            if key.modifiers.contains(KeyModifiers::SHIFT) && app.folder_form.focus != 0 =>
        {
            try_paste_into(&mut app.folder_form.fields, app.folder_form.focus);
        }
        KeyCode::Enter => {
            // Folder ID is read-only; field[0] kept for display only
            let path = app.folder_form.fields[1].trim();

            if let Err(e) = syncthing_core::validation::validate_path(path) {
                app.popup = Popup::Error(e.to_string());
                return false;
            }

            let selected = app.folder_selected;
            if let Some(folder) = app.config.folders.get_mut(selected) {
                folder.path = path.to_string();
                // Rebuild devices list from selection + local_id
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
                app.popup = Popup::None;
                save_and_log(app);
            }
        }
        KeyCode::Down => {
            if app.folder_form.focus == device_list_focus
                && app.folder_device_selected + 1 < app.config.devices.len()
            {
                app.folder_device_selected += 1;
            } else {
                app.folder_form.focus = ((app.folder_form.focus + 1).max(1)) % (max_focus + 1);
                if app.folder_form.focus == 0 {
                    app.folder_form.focus = 1;
                }
            }
        }
        KeyCode::Up => {
            if app.folder_form.focus == device_list_focus && app.folder_device_selected > 0 {
                app.folder_device_selected -= 1;
            } else if app.folder_form.focus <= 1 {
                app.folder_form.focus = max_focus;
            } else {
                app.folder_form.focus -= 1;
            }
        }
        KeyCode::Tab => {
            app.folder_form.focus = ((app.folder_form.focus + 1).max(1)) % (max_focus + 1);
            if app.folder_form.focus == 0 {
                app.folder_form.focus = 1;
            }
        }
        KeyCode::BackTab => {
            if app.folder_form.focus <= 1 {
                app.folder_form.focus = max_focus;
            } else {
                app.folder_form.focus -= 1;
            }
        }
        KeyCode::Char(' ') if app.folder_form.focus == device_list_focus => {
            if let Some(selected) = app
                .folder_device_selection
                .get_mut(app.folder_device_selected)
            {
                *selected = !*selected;
            }
        }
        KeyCode::Char(c) if app.folder_form.focus != 0 => {
            if let Some(field) = app.folder_form.fields.get_mut(app.folder_form.focus) {
                field.push(c);
            }
        }
        KeyCode::Backspace if app.folder_form.focus != 0 => {
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
        Ok(_) => {
            app.push_log("Config saved.".to_string());
            // 通知运行中的 sync_service 配置已变更
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
