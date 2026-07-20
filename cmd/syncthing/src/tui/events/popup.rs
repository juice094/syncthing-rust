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
    app.config.folders.push(folder);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::Tab;
    use crate::tui::forms::FormState;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use syncthing_core::types::Config;

    fn test_app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = App::new(
            dir.path().to_path_buf(),
            "tcp://0.0.0.0:22001".to_string(),
            "test-node".to_string(),
            Config::default(),
        );
        (dir, app)
    }

    /// 回归：AddFolder 提交后新文件夹必须真正写入 config（曾缺失 push 导致静默丢弃）
    #[test]
    fn test_submit_add_folder_persists_folder() {
        let (_dir, mut app) = test_app();
        app.tab = Tab::Folders;
        let form = FormState::new("Add Folder", 60, 16)
            .add_field(
                "folder_id",
                "Folder ID",
                "test-folder".to_string(),
                true,
                None,
            )
            .add_field("path", "Path", "C:\\sync\\test".to_string(), true, None);
        app.form = Some(form);
        app.popup = Popup::AddFolder;

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        handle_popup_key(&mut app, key);

        assert_eq!(app.popup, Popup::None, "提交成功后弹窗应关闭");
        assert_eq!(app.config.folders.len(), 1, "新文件夹必须写入 config");
        assert_eq!(app.config.folders[0].id, "test-folder");
        assert_eq!(app.config.folders[0].path, "C:\\sync\\test");
        assert!(
            _dir.path().join("config.json").exists(),
            "配置应落盘到 config.json"
        );
    }

    /// 回归：非法 folder ID 应停留在表单并显示错误，不得写入 config
    #[test]
    fn test_submit_add_folder_invalid_id_rejected() {
        let (_dir, mut app) = test_app();
        app.tab = Tab::Folders;
        let form = FormState::new("Add Folder", 60, 16)
            .add_field("folder_id", "Folder ID", "bad id!".to_string(), true, None)
            .add_field("path", "Path", "C:\\sync\\test".to_string(), true, None);
        app.form = Some(form);
        app.popup = Popup::AddFolder;

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        handle_popup_key(&mut app, key);

        assert!(app.config.folders.is_empty(), "非法输入不得写入 config");
        assert_eq!(app.popup, Popup::AddFolder, "校验失败应停留在表单");
    }

    use std::str::FromStr;
    use std::sync::{Arc, Mutex};
    use syncthing_core::types::Device;

    fn valid_device_id() -> String {
        "YTKWHNG-OT27ZGH-6VVBRIJ-OHOUNWT-DYLJ2NR-TCXUXHI-QDUQR2U-OPLCBQG".to_string()
    }

    fn make_device(id: DeviceId) -> Device {
        Device {
            id,
            name: None,
            addresses: vec![AddressType::Dynamic],
            paused: false,
            introducer: false,
        }
    }

    fn device_form(title: &'static str, id: &str, name: &str, addr: &str) -> FormState {
        FormState::new(title, 72, 14)
            .add_field("device_id", "Device ID", id.to_string(), true, None)
            .add_field("device_name", "Name", name.to_string(), true, None)
            .add_field("address", "Address", addr.to_string(), true, None)
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    #[test]
    fn test_submit_add_device_persists_device() {
        let (_dir, mut app) = test_app();
        app.tab = Tab::Devices;
        app.form = Some(device_form("Add Device", &valid_device_id(), "peer", ""));
        app.popup = Popup::AddDevice;

        handle_popup_key(&mut app, enter());

        assert_eq!(app.popup, Popup::None);
        assert_eq!(app.config.devices.len(), 1, "新设备必须写入 config");
        assert_eq!(app.config.devices[0].name.as_deref(), Some("peer"));
        assert!(matches!(
            app.config.devices[0].addresses[0],
            AddressType::Dynamic
        ));
        assert!(_dir.path().join("config.json").exists(), "配置应落盘");
    }

    #[test]
    fn test_submit_add_device_invalid_id_rejected() {
        let (_dir, mut app) = test_app();
        app.tab = Tab::Devices;
        app.form = Some(device_form("Add Device", "not-a-device-id", "peer", ""));
        app.popup = Popup::AddDevice;

        handle_popup_key(&mut app, enter());

        assert!(app.config.devices.is_empty(), "非法输入不得写入 config");
        assert_eq!(app.popup, Popup::AddDevice, "校验失败应停留在表单");
    }

    #[test]
    fn test_submit_edit_device_updates_fields() {
        let (_dir, mut app) = test_app();
        let id = DeviceId::from_str(&valid_device_id()).expect("device id");
        app.config.devices.push(make_device(id));
        app.tab = Tab::Devices;
        app.device_selected = 0;
        app.form = Some(device_form(
            "Edit Device",
            &valid_device_id(),
            "renamed",
            "tcp://1.2.3.4:22001",
        ));
        app.popup = Popup::EditDevice;

        handle_popup_key(&mut app, enter());

        assert_eq!(app.popup, Popup::None);
        assert_eq!(app.config.devices[0].name.as_deref(), Some("renamed"));
        assert!(
            matches!(&app.config.devices[0].addresses[0], AddressType::Tcp(a) if a == "tcp://1.2.3.4:22001")
        );
    }

    #[test]
    fn test_submit_edit_folder_updates_path_and_shares() {
        let (_dir, mut app) = test_app();
        let id = DeviceId::from_str(&valid_device_id()).expect("device id");
        app.config.devices.push(make_device(id));
        let mut folder = Folder::new("f1", "C:\\old");
        folder.devices.push(id);
        app.config.folders.push(folder);
        app.tab = Tab::Folders;
        app.folder_selected = 0;
        app.folder_device_selection = vec![true];
        let form = FormState::new("Edit Folder", 60, 16)
            .add_field("folder_id", "Folder ID", "f1".to_string(), false, None)
            .add_field("path", "Path", "C:\\new".to_string(), true, None);
        app.form = Some(form);
        app.popup = Popup::EditFolder;

        handle_popup_key(&mut app, enter());

        assert_eq!(app.popup, Popup::None);
        assert_eq!(app.config.folders[0].path, "C:\\new");
        assert!(
            app.config.folders[0].devices.contains(&id),
            "勾选设备应保留在共享列表"
        );
    }

    /// 记录 update_config 调用的 mock，验证 TUI 配置变更会热更新到运行中的同步服务
    struct MockSyncManager {
        publisher: syncthing_sync::EventPublisher,
        updated: Arc<Mutex<Vec<Config>>>,
    }

    impl MockSyncManager {
        fn new(updated: Arc<Mutex<Vec<Config>>>) -> Self {
            Self {
                publisher: syncthing_sync::EventPublisher::default(),
                updated,
            }
        }
    }

    #[async_trait::async_trait]
    impl syncthing_sync::SyncManager for MockSyncManager {
        async fn get_config(&self) -> syncthing_sync::Result<Config> {
            Ok(Config::default())
        }
        async fn update_config(&self, config: Config) -> syncthing_sync::Result<()> {
            self.updated.lock().expect("lock").push(config);
            Ok(())
        }
        async fn add_device(&self, _device: Device) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn remove_device(&self, _device_id: &DeviceId) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn add_folder(&self, _folder: Folder) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn remove_folder(&self, _folder_id: &str) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn get_folder_state(
            &self,
            folder_id: &str,
        ) -> syncthing_sync::Result<syncthing_sync::FolderState> {
            Ok(syncthing_sync::FolderState::new(folder_id))
        }
        async fn start(&self) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn stop(&self) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn scan_folder(&self, _folder_id: &str) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn scan_folder_sub(
            &self,
            _folder_id: &str,
            _sub: &str,
        ) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn pull_folder(&self, _folder_id: &str) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn get_connected_devices(&self) -> syncthing_sync::Result<Vec<DeviceId>> {
            Ok(Vec::new())
        }
        async fn connect_device(&self, _device_id: DeviceId) -> syncthing_sync::Result<()> {
            Ok(())
        }
        async fn disconnect_device(&self, _device_id: DeviceId) -> syncthing_sync::Result<()> {
            Ok(())
        }
        fn subscribe_events(&self) -> syncthing_sync::EventSubscriber {
            self.publisher.subscribe()
        }
        async fn get_stats(&self) -> syncthing_sync::Result<syncthing_sync::model::SyncStats> {
            Ok(syncthing_sync::model::SyncStats::default())
        }
    }

    #[tokio::test]
    async fn test_submit_add_folder_notifies_sync_service() {
        let (_dir, mut app) = test_app();
        let updated = Arc::new(Mutex::new(Vec::new()));
        app.sync_service = Some(Arc::new(MockSyncManager::new(Arc::clone(&updated))));
        app.tab = Tab::Folders;
        let form = FormState::new("Add Folder", 60, 16)
            .add_field(
                "folder_id",
                "Folder ID",
                "hot-folder".to_string(),
                true,
                None,
            )
            .add_field("path", "Path", "C:\\sync\\hot".to_string(), true, None);
        app.form = Some(form);
        app.popup = Popup::AddFolder;

        handle_popup_key(&mut app, enter());

        // save_and_log 中的 update_config 是 fire-and-forget spawn，轮询等待其完成
        for _ in 0..100 {
            if !updated.lock().expect("lock").is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let calls = updated.lock().expect("lock");
        assert_eq!(calls.len(), 1, "配置变更必须通知运行中的 sync service");
        assert_eq!(calls[0].folders.len(), 1);
        assert_eq!(calls[0].folders[0].id, "hot-folder");
    }
}
