//! Windows 系统托盘（Win32 API），参照 tray-icon / systray 标准实现。
//!
//! TrayApp::spawn() 在独立线程创建隐藏窗口 + 托盘图标 + 消息循环。
//! 通过 mpsc::Receiver 向主线程发送托盘事件。

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu,
    HICON, HMENU, IDI_APPLICATION, IDI_ERROR, IDI_INFORMATION, IDI_WARNING, LR_DEFAULTCOLOR,
    MF_STRING, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_COMMAND, WM_DESTROY, WM_LBUTTONDOWN, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    OpenWebUi,
    OpenTui,
    ToggleDaemon,
    Exit,
}

const WM_TRAYICON: u32 = 1024 + 1;
const ID_OPEN_WEBUI: u32 = 1001;
const ID_OPEN_TUI: u32 = 1004;
const ID_TOGGLE_DAEMON: u32 = 1002;
const ID_EXIT: u32 = 1003;
const TRAY_ICON_ID: u32 = 1;

// Balloon notification flags (missing from windows crate 0.58 feature set)
const NIF_INFO: u32 = 0x0000_0010;
const NIIF_NONE: u32 = 0x0000_0000;
const NIIF_INFO: u32 = 0x0000_0001;
const NIIF_WARNING: u32 = 0x0000_0002;
const NIIF_ERROR: u32 = 0x0000_0003;

/// Balloon icon severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BalloonIcon {
    None,
    Info,
    Warning,
    Error,
}

impl BalloonIcon {
    fn to_niif(self) -> u32 {
        match self {
            BalloonIcon::None => NIIF_NONE,
            BalloonIcon::Info => NIIF_INFO,
            BalloonIcon::Warning => NIIF_WARNING,
            BalloonIcon::Error => NIIF_ERROR,
        }
    }
}

/// 线程安全的状态存储（替代 static mut）
static STATE: OnceLock<TrayState> = OnceLock::new();

struct TrayState {
    event_tx: Sender<TrayEvent>,
    hwnd: isize,
    /// Kept for TaskbarCreated message comparison; static analysis cannot see the use.
    #[allow(dead_code)]
    taskbar_created_msg: u32,
    /// Custom HICON stored as usize because raw pointers are neither Send nor Sync.
    custom_hicon: Option<usize>,
}

pub fn spawn() -> Receiver<TrayEvent> {
    let (tx, rx) = channel::<TrayEvent>();
    std::thread::spawn(move || unsafe {
        if let Err(e) = tray_thread(tx) {
            // 无法输出到控制台，静默失败
            let _ = e;
        }
    });
    rx
}

/// 返回托盘窗口句柄（供 update_icon / update_tooltip 使用）
pub fn tray_hwnd() -> HWND {
    HWND(STATE.get().map(|s| s.hwnd).unwrap_or(0) as *mut _)
}

// 由 build.rs 生成的 ICO 数据（32x32 32bpp ARGB 硬盘图标）
static ICON_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon.ico"));

/// 从 build.rs 生成的 ICO 加载自定义图标。
/// 跳过 6 字节 ICONDIR + 16 字节 ICONDIRENTRY，指向 BITMAPINFOHEADER。
unsafe fn load_custom_icon() -> Option<HICON> {
    const ICONDIR_SIZE: usize = 6;
    const ENTRY_SIZE: usize = 16;
    if ICON_BYTES.len() < ICONDIR_SIZE + ENTRY_SIZE {
        return None;
    }
    let offset_bytes = [
        ICON_BYTES[ICONDIR_SIZE + 12],
        ICON_BYTES[ICONDIR_SIZE + 13],
        ICON_BYTES[ICONDIR_SIZE + 14],
        ICON_BYTES[ICONDIR_SIZE + 15],
    ];
    let image_offset = u32::from_le_bytes(offset_bytes) as usize;
    let image_data = ICON_BYTES.get(image_offset..)?;
    CreateIconFromResourceEx(
        image_data,
        windows::Win32::Foundation::BOOL(1),
        0x0003_0000,
        32,
        32,
        LR_DEFAULTCOLOR,
    )
    .ok()
}

unsafe fn tray_thread(tx: Sender<TrayEvent>) -> anyhow::Result<()> {
    // 注册 TaskbarCreated 消息（explorer.exe 重启后重新添加图标）
    let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));

    let hmodule = GetModuleHandleW(None)?;
    let hinstance = HINSTANCE(hmodule.0);

    let class_name = w!("SyncthingTrayV2");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: hinstance,
        lpszClassName: class_name,
        ..Default::default()
    };
    RegisterClassW(&wc);

    // 隐藏窗口：零尺寸 + WS_EX_TOOLWINDOW（不出现于 Alt+Tab / 任务栏）
    let hwnd = CreateWindowExW(
        WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        class_name,
        w!(""),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        None,
        None,
        hinstance,
        None,
    )?;

    // 优先使用 build.rs 生成的自定义图标；失败时回退到系统图标
    let custom_hicon = load_custom_icon();
    let hicon = custom_hicon
        .map(|h| HICON(h.0))
        .unwrap_or_else(|| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAYICON,
        hIcon: hicon,
        ..Default::default()
    };
    let tip = w!("syncthing-rust");
    let tw: &[u16] = tip.as_wide();
    let tl = tw.len().min(nid.szTip.len() - 1);
    nid.szTip[..tl].copy_from_slice(&tw[..tl]);

    if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
        anyhow::bail!("NIM_ADD failed");
    }

    // 初始化全局状态
    let _ = STATE.set(TrayState {
        event_tx: tx,
        hwnd: hwnd.0 as isize,
        taskbar_created_msg,
        custom_hicon: custom_hicon.map(|h| h.0 as usize),
    });

    // 消息循环
    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).into() {
        // TaskbarCreated: explorer 重启后重新注册托盘图标
        if msg.message == taskbar_created_msg {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
        DispatchMessageW(&msg);
    }

    Ok(())
}

unsafe fn show_context_menu(hwnd: HWND) {
    let hmenu = CreatePopupMenu().unwrap_or(HMENU(std::ptr::null_mut()));
    if hmenu.0.is_null() {
        return;
    }
    let _ = AppendMenuW(hmenu, MF_STRING, ID_OPEN_WEBUI as usize, w!("Open Web UI"));
    let _ = AppendMenuW(hmenu, MF_STRING, ID_OPEN_TUI as usize, w!("Open TUI"));
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        ID_TOGGLE_DAEMON as usize,
        w!("Start / Stop Daemon"),
    );
    let _ = AppendMenuW(hmenu, MF_STRING, ID_EXIT as usize, w!("Exit"));
    let _ = SetForegroundWindow(hwnd);
    let mut pt = Default::default();
    let _ = GetCursorPos(&mut pt);
    let _ = TrackPopupMenu(
        hmenu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );
    // Shell 规范：菜单关闭后发送 WM_NULL
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(hmenu);
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            match lparam.0 as u32 {
                WM_RBUTTONUP => show_context_menu(hwnd),
                WM_LBUTTONDOWN => {
                    if let Some(s) = STATE.get() {
                        let _ = s.event_tx.send(TrayEvent::OpenTui);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 as u32;
            if let Some(s) = STATE.get() {
                let event = match id {
                    ID_OPEN_WEBUI => Some(TrayEvent::OpenWebUi),
                    ID_OPEN_TUI => Some(TrayEvent::OpenTui),
                    ID_TOGGLE_DAEMON => Some(TrayEvent::ToggleDaemon),
                    ID_EXIT => Some(TrayEvent::Exit),
                    _ => None,
                };
                if let Some(e) = event {
                    let _ = s.event_tx.send(e);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // 清理托盘图标
            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: TRAY_ICON_ID,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            // 释放自定义图标 GDI 资源
            if let Some(s) = STATE.get() {
                if let Some(ptr) = s.custom_hicon {
                    let _ = DestroyIcon(HICON(ptr as *mut _));
                }
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ==================== update helpers ====================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconType {
    Default,
    Idle,
    Syncing,
    Error,
}

impl IconType {
    fn to_system_icon(self) -> PCWSTR {
        match self {
            IconType::Default => IDI_APPLICATION,
            IconType::Idle => IDI_INFORMATION,
            IconType::Syncing => IDI_WARNING,
            IconType::Error => IDI_ERROR,
        }
    }
}

pub unsafe fn update_icon(_hwnd: HWND, icon_type: IconType) {
    let Some(state) = STATE.get() else { return };

    // Default/Idle 使用 build.rs 生成的自定义图标；Error/Syncing 使用系统图标（颜色区分）
    let hicon = match icon_type {
        IconType::Default | IconType::Idle => state
            .custom_hicon
            .map(|p| HICON(p as *mut _))
            .unwrap_or_else(|| LoadIconW(None, IDI_APPLICATION).unwrap_or_default()),
        IconType::Syncing | IconType::Error => match LoadIconW(None, icon_type.to_system_icon()) {
            Ok(h) => h,
            Err(_) => return,
        },
    };

    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(state.hwnd as *mut _),
        uID: TRAY_ICON_ID,
        uFlags: NIF_ICON,
        hIcon: hicon,
        ..Default::default()
    };
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}

pub unsafe fn update_tooltip(_hwnd: HWND, text: &str) {
    let Some(state) = STATE.get() else { return };
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(state.hwnd as *mut _),
        uID: TRAY_ICON_ID,
        uFlags: NIF_TIP,
        ..Default::default()
    };
    let wide: Vec<u16> = text.encode_utf16().collect();
    let len = wide.len().min(nid.szTip.len() - 1);
    nid.szTip[..len].copy_from_slice(&wide[..len]);
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}

/// Show a Windows tray balloon notification.
///
/// Title truncated to 63 UTF-16 code units; text truncated to 255 UTF-16 code units
/// (NOTIFYICONDATAW limits on Win11).
pub unsafe fn show_notification(_hwnd: HWND, title: &str, text: &str, icon: BalloonIcon) {
    let Some(state) = STATE.get() else { return };
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(state.hwnd as *mut _),
        uID: TRAY_ICON_ID,
        uFlags: windows::Win32::UI::Shell::NOTIFY_ICON_DATA_FLAGS(NIF_INFO),
        ..Default::default()
    };

    let title_wide: Vec<u16> = title.encode_utf16().collect();
    let text_wide: Vec<u16> = text.encode_utf16().collect();

    let title_len = title_wide
        .len()
        .min(nid.szInfoTitle.len().saturating_sub(1));
    nid.szInfoTitle[..title_len].copy_from_slice(&title_wide[..title_len]);

    let text_len = text_wide.len().min(nid.szInfo.len().saturating_sub(1));
    nid.szInfo[..text_len].copy_from_slice(&text_wide[..text_len]);

    nid.dwInfoFlags = windows::Win32::UI::Shell::NOTIFY_ICON_INFOTIP_FLAGS(icon.to_niif());

    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}
