use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};

use crate::save_config;
use crate::tui::app::{App, Popup, Tab};
use crate::tui::forms::FormState;

/// Tab 非弹窗模式下的按键处理
pub fn handle_tab_key(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Right | KeyCode::Tab => app.tab = app.tab.next(),
        KeyCode::Left | KeyCode::BackTab => app.tab = app.tab.prev(),

        KeyCode::Char('/') => match app.tab {
            Tab::Devices | Tab::Folders => {
                let initial = app.list_filter.clone().unwrap_or_default();
                app.popup = Popup::Filter { query: initial };
            }
            _ => {}
        },

        KeyCode::Esc => match app.tab {
            Tab::Devices | Tab::Folders => {
                app.list_filter = None;
                app.list_filter_selected = 0;
                app.recompute_device_filter_matches();
                app.recompute_folder_filter_matches();
            }
            _ => {}
        },

        KeyCode::Char('a') | KeyCode::Insert => match app.tab {
            Tab::Devices => {
                let form = FormState::new("Add Device", 72, 14)
                    .add_field("device_id", "Device ID", String::new(), true, None)
                    .add_field("device_name", "Name", String::new(), true, None)
                    .add_field("address", "Address", String::new(), true, None);
                app.form = Some(form);
                app.popup = Popup::AddDevice;
            }
            Tab::Folders => {
                let form = FormState::new("Add Folder", 60, 16)
                    .add_field("folder_id", "Folder ID", String::new(), true, None)
                    .add_field(
                        "path",
                        "Path",
                        String::new(),
                        true,
                        Some("Absolute path, e.g. C:\\Users\\me\\sync"),
                    );
                app.resize_form();
                app.form = Some(form);
                // 重置共享设备选择，避免残留上一次 EditFolder 的勾选
                app.folder_device_selected = 0;
                app.folder_device_selection = vec![false; app.config.devices.len()];
                app.popup = Popup::AddFolder;
            }
            _ => {}
        },

        KeyCode::Char('d') | KeyCode::Delete => match app.tab {
            Tab::Devices if !app.device_filter_matches.is_empty() => {
                let device = &app.config.devices[app.device_selected];
                let id = device.id;
                let name = device.name.clone().unwrap_or_default();
                let id_short = device.id.short_id();
                app.confirm_callback = Some(Box::new(move |app| {
                    app.config.devices.retain(|d| d.id != id);
                    for folder in &mut app.config.folders {
                        folder.devices.retain(|&did| did != id);
                    }
                    app.resize_form();
                    save_and_log(app);
                }));
                app.popup = Popup::Confirm {
                    title: "Confirm Delete".to_string(),
                    message: format!(r#"Delete device "{name}" ({id_short})?"#),
                };
            }
            Tab::Folders if !app.folder_filter_matches.is_empty() => {
                let folder_id = app.config.folders[app.folder_selected].id.clone();
                let folder_id_for_closure = folder_id.clone();
                app.confirm_callback = Some(Box::new(move |app| {
                    app.config.folders.retain(|f| f.id != folder_id_for_closure);
                    save_and_log(app);
                }));
                app.popup = Popup::Confirm {
                    title: "Confirm Delete".to_string(),
                    message: format!(r#"Delete folder "{folder_id}"?"#),
                };
            }
            _ => {}
        },

        KeyCode::Down => match app.tab {
            Tab::Devices if app.list_filter_selected + 1 < app.device_filter_matches.len() => {
                app.list_filter_selected += 1;
                app.device_selected = app.device_filter_matches[app.list_filter_selected];
            }
            Tab::Folders if app.list_filter_selected + 1 < app.folder_filter_matches.len() => {
                app.list_filter_selected += 1;
                app.folder_selected = app.folder_filter_matches[app.list_filter_selected];
            }
            _ => {}
        },

        KeyCode::Up => match app.tab {
            Tab::Devices if app.list_filter_selected > 0 => {
                app.list_filter_selected -= 1;
                app.device_selected = app.device_filter_matches[app.list_filter_selected];
            }
            Tab::Folders if app.list_filter_selected > 0 => {
                app.list_filter_selected -= 1;
                app.folder_selected = app.folder_filter_matches[app.list_filter_selected];
            }
            _ => {}
        },

        KeyCode::Enter | KeyCode::Char('e') => match app.tab {
            Tab::Devices if !app.device_filter_matches.is_empty() => {
                if let Some(device) = app.config.devices.get(app.device_selected) {
                    let addr = device
                        .addresses
                        .first()
                        .map(|a| a.to_string())
                        .unwrap_or_default();
                    let form = FormState::new("Edit Device", 72, 14)
                        .add_field("device_id", "Device ID", device.id.to_string(), false, None)
                        .add_field(
                            "device_name",
                            "Name",
                            device.name.clone().unwrap_or_default(),
                            true,
                            None,
                        )
                        .add_field("address", "Address", addr, true, None);
                    app.form = Some(form);
                    app.popup = Popup::EditDevice;
                }
            }
            Tab::Folders if !app.folder_filter_matches.is_empty() => {
                if let Some(folder) = app.config.folders.get(app.folder_selected) {
                    let form = FormState::new("Edit Folder", 60, 16)
                        .add_field(
                            "folder_id",
                            "Folder ID",
                            folder.id.clone(),
                            false,
                            Some("Folder ID cannot be changed"),
                        )
                        .add_field(
                            "path",
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

        KeyCode::Char('i') if app.tab == Tab::Folders && !app.folder_filter_matches.is_empty() => {
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

/// 保存配置并刷新相关状态
pub fn save_and_log(app: &mut App) {
    let path = app.config_dir.join("config.json");
    match save_config(&path, &app.config) {
        Ok(_) => {
            app.push_log("Config saved.".to_string());
            app.recompute_device_filter_matches();
            app.recompute_folder_filter_matches();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use std::str::FromStr;
    use syncthing_core::types::{AddressType, Config, Device};
    use syncthing_core::DeviceId;

    const VALID_ID: &str = "YTKWHNG-OT27ZGH-6VVBRIJ-OHOUNWT-DYLJ2NR-TCXUXHI-QDUQR2U-OPLCBQG";

    /// 回归：打开 AddFolder 时共享设备勾选必须重置，
    /// 避免残留上一次 EditFolder 的选择导致新文件夹被意外共享
    #[test]
    fn test_open_add_folder_resets_device_selection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let id = DeviceId::from_str(VALID_ID).expect("device id");
        let mut config = Config::default();
        config.devices.push(Device {
            id,
            name: None,
            addresses: vec![AddressType::Dynamic],
            paused: false,
            introducer: false,
        });
        let mut app = App::new(
            dir.path().to_path_buf(),
            "tcp://0.0.0.0:22001".to_string(),
            "test-node".to_string(),
            config,
        );
        app.tab = Tab::Folders;
        // 模拟上一次 EditFolder 残留的勾选状态
        app.folder_device_selection = vec![true];
        app.folder_device_selected = 0;

        handle_tab_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        );

        assert_eq!(app.popup, Popup::AddFolder);
        assert_eq!(
            app.folder_device_selection,
            vec![false],
            "AddFolder 打开时勾选状态必须重置"
        );
    }
}
