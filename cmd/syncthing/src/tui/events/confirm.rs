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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::Tab;
    use crate::tui::events::tab::handle_tab_key;
    use crossterm::event::KeyModifiers;
    use std::str::FromStr;
    use syncthing_core::types::{AddressType, Config, Device, Folder};
    use syncthing_core::DeviceId;

    const VALID_ID: &str = "YTKWHNG-OT27ZGH-6VVBRIJ-OHOUNWT-DYLJ2NR-TCXUXHI-QDUQR2U-OPLCBQG";

    /// 构造含 1 台远程设备 + 1 个共享给该设备的文件夹的 App
    fn app_with_device_and_folder() -> (tempfile::TempDir, App, DeviceId) {
        let dir = tempfile::tempdir().expect("tempdir");
        let id = DeviceId::from_str(VALID_ID).expect("device id");
        let mut config = Config::default();
        config.devices.push(Device {
            id,
            name: Some("peer".to_string()),
            addresses: vec![AddressType::Dynamic],
            paused: false,
            introducer: false,
        });
        let mut folder = Folder::new("f1", "C:\\sync\\f1");
        folder.devices.push(id);
        config.folders.push(folder);
        let app = App::new(
            dir.path().to_path_buf(),
            "tcp://0.0.0.0:22001".to_string(),
            "test-node".to_string(),
            config,
        );
        (dir, app, id)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_delete_device_via_confirm() {
        let (_dir, mut app, id) = app_with_device_and_folder();
        app.tab = Tab::Devices;

        handle_tab_key(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.popup, Popup::Confirm { .. }), "应弹出确认框");

        handle_confirm_key(&mut app, key(KeyCode::Char('y')));
        assert_eq!(app.popup, Popup::None);
        assert!(app.config.devices.is_empty(), "设备应被删除");
        assert!(
            !app.config.folders[0].devices.contains(&id),
            "文件夹共享列表应同步移除该设备"
        );
        assert!(_dir.path().join("config.json").exists(), "配置应落盘");
    }

    #[test]
    fn test_delete_device_cancelled_by_n() {
        let (_dir, mut app, id) = app_with_device_and_folder();
        app.tab = Tab::Devices;

        handle_tab_key(&mut app, key(KeyCode::Char('d')));
        handle_confirm_key(&mut app, key(KeyCode::Char('n')));

        assert_eq!(app.popup, Popup::None);
        assert_eq!(app.config.devices.len(), 1, "取消删除不得改动 config");
        assert!(app.config.folders[0].devices.contains(&id));
    }

    #[test]
    fn test_delete_folder_via_confirm() {
        let (_dir, mut app, _id) = app_with_device_and_folder();
        app.tab = Tab::Folders;

        handle_tab_key(&mut app, key(KeyCode::Char('d')));
        assert!(matches!(app.popup, Popup::Confirm { .. }));

        handle_confirm_key(&mut app, key(KeyCode::Char('y')));
        assert_eq!(app.popup, Popup::None);
        assert!(app.config.folders.is_empty(), "文件夹应被删除");
        assert!(_dir.path().join("config.json").exists(), "配置应落盘");
    }
}
