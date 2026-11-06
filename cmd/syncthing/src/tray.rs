//! Windows system tray (Win32 API), modeled on the tray-icon / systray standard.
//!
//! `spawn()` creates the hidden window, tray icon, and message loop on a
//! dedicated thread and streams tray events to the main thread over an mpsc
//! channel.

use std::sync::atomic::{AtomicBool, Ordering};
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
    DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics,
    LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetForegroundWindow, TrackPopupMenu, HICON, HMENU, IDI_APPLICATION, IDI_ERROR, IDI_INFORMATION,
    IDI_WARNING, LR_DEFAULTCOLOR, MENU_ITEM_FLAGS, MF_GRAYED, MF_STRING, SM_CXSMICON,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_OVERLAPPED,
};

/// Guards against a double DestroyIcon: cleanup() sets this after releasing,
/// and WM_DESTROY checks it before releasing again.
static CLEANED_UP: AtomicBool = AtomicBool::new(false);

/// Current daemon running state, read by the tray thread while rendering menus.
/// Written by the main thread on daemon start/stop.
static DAEMON_RUNNING: AtomicBool = AtomicBool::new(false);

/// Updates the daemon running state (called from the main thread).
pub fn set_daemon_running(running: bool) {
    DAEMON_RUNNING.store(running, Ordering::Release);
}

/// Reads whether the daemon is running (called from the tray thread).
pub fn daemon_running() -> bool {
    DAEMON_RUNNING.load(Ordering::Acquire)
}

/// Events delivered from the tray thread to the main thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    OpenTui,
    ToggleDaemon,
    Exit,
}

const WM_TRAYICON: u32 = WM_APP + 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balloon_icon_to_niif() {
        assert_eq!(BalloonIcon::None.to_niif(), NIIF_NONE);
        assert_eq!(BalloonIcon::Info.to_niif(), NIIF_INFO);
        assert_eq!(BalloonIcon::Warning.to_niif(), NIIF_WARNING);
        assert_eq!(BalloonIcon::Error.to_niif(), NIIF_ERROR);
    }

    #[test]
    fn test_icon_type_system_mapping() {
        // 系统图标选择仅用于 Error/Syncing；Default/Idle 优先使用自定义图标。
        assert!(IconType::Default.to_system_icon() == IDI_APPLICATION);
        assert!(IconType::Idle.to_system_icon() == IDI_INFORMATION);
        assert!(IconType::Syncing.to_system_icon() == IDI_WARNING);
        assert!(IconType::Error.to_system_icon() == IDI_ERROR);
    }

    #[test]
    fn test_tray_event_variants() {
        // 事件枚举在 Windows 消息循环和主线程之间传递，需保证 Copy + Eq。
        let e1 = TrayEvent::OpenTui;
        let e2 = TrayEvent::OpenTui;
        assert_eq!(e1, e2);
        assert_ne!(e1, TrayEvent::Exit);
    }
}

/// Thread-safe state store (replaces a static mut).
static STATE: OnceLock<TrayState> = OnceLock::new();

struct TrayState {
    event_tx: Sender<TrayEvent>,
    hwnd: isize,
    /// Kept for TaskbarCreated message comparison; static analysis cannot see the use.
    #[allow(dead_code)]
    taskbar_created_msg: u32,
    /// Current icon HICON (any state), stored as usize because the raw pointer is not Send/Sync.
    current_hicon: Option<usize>,
}

/// Windows error message box (for the no-console case).
unsafe fn show_error_message(title: &str, message: &str) {
    use std::iter::once;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title_wide: Vec<u16> = title.encode_utf16().chain(once(0)).collect();
    let msg_wide: Vec<u16> = message.encode_utf16().chain(once(0)).collect();
    let _ = MessageBoxW(
        None,
        PCWSTR(msg_wide.as_ptr()),
        PCWSTR(title_wide.as_ptr()),
        MB_OK | MB_ICONERROR,
    );
}

/// Starts the tray icon and message loop on a dedicated Win32 thread.
///
/// Returns `(Receiver<TrayEvent>, JoinHandle)` so the main thread can consume
/// events and join the thread on exit.
///
/// SAFETY: internally spawns `tray_thread` via `std::thread::spawn`, which
/// guarantees the message-loop window is only created once.
pub fn spawn() -> (Receiver<TrayEvent>, std::thread::JoinHandle<()>) {
    let (tx, rx) = channel::<TrayEvent>();
    let handle = std::thread::spawn(move || {
        // SAFETY: tray_thread must run on the Win32 thread that owns the
        // message-loop window. This closure runs on the OS thread created by
        // `std::thread::spawn`, which satisfies the requirement.
        unsafe {
            if let Err(e) = tray_thread(tx) {
                tracing::error!("Tray thread error: {}", e);
            }
        }
    });
    (rx, handle)
}

/// Removes the tray icon (call before Exit).
///
/// SAFETY: only called from the main-thread Exit path, after the icon was
/// registered via NIM_ADD. Shell_NotifyIconW and DestroyIcon are thread-safe
/// system calls. Posts WM_CLOSE so the tray thread leaves its message loop.
pub fn cleanup() {
    if let Some(s) = STATE.get() {
        // SAFETY: STATE was initialized via OnceLock, so hwnd/current_hicon are
        // valid. NIM_DELETE is a thread-safe Win32 API.
        unsafe {
            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: HWND(s.hwnd as *mut _),
                uID: TRAY_ICON_ID,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            // SAFETY: current_hicon was created by CreateIconFromResourceEx and
            // must be released with DestroyIcon.
            if let Some(ptr) = s.current_hicon {
                let _ = DestroyIcon(HICON(ptr as *mut _));
            }
            CLEANED_UP.store(true, Ordering::Release);
            // PostMessage(WM_CLOSE) -> DefWindowProc -> DestroyWindow -> WM_DESTROY -> PostQuitMessage
            let _ = PostMessageW(HWND(s.hwnd as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Returns the tray window handle (for update_icon / update_tooltip).
pub fn tray_hwnd() -> HWND {
    HWND(STATE.get().map(|s| s.hwnd).unwrap_or(0) as *mut _)
}

// Multi-size ICO data generated by build.rs (16/24/32/48/256)
static ICON_BYTES_DEFAULT: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon-default.ico"));
static ICON_BYTES_IDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon-idle.ico"));
static ICON_BYTES_SYNCING: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon-syncing.ico"));
static ICON_BYTES_ERROR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon-error.ico"));

fn icon_bytes(icon_type: IconType) -> &'static [u8] {
    match icon_type {
        IconType::Default => ICON_BYTES_DEFAULT,
        IconType::Idle => ICON_BYTES_IDLE,
        IconType::Syncing => ICON_BYTES_SYNCING,
        IconType::Error => ICON_BYTES_ERROR,
    }
}

struct IcoEntry {
    width: u32,
    height: u32,
    size: u32,
    offset: u32,
}

fn parse_ico(data: &[u8]) -> Option<Vec<IcoEntry>> {
    if data.len() < 6 {
        return None;
    }
    let count = u16::from_le_bytes([data[4], data[5]]) as usize;
    if data.len() < 6 + count * 16 {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let base = 6 + i * 16;
        let width = if data[base] == 0 {
            256
        } else {
            data[base] as u32
        };
        let height = if data[base + 1] == 0 {
            256
        } else {
            data[base + 1] as u32
        };
        let size = u32::from_le_bytes([
            data[base + 8],
            data[base + 9],
            data[base + 10],
            data[base + 11],
        ]);
        let offset = u32::from_le_bytes([
            data[base + 12],
            data[base + 13],
            data[base + 14],
            data[base + 15],
        ]);
        entries.push(IcoEntry {
            width,
            height,
            size,
            offset,
        });
    }
    Some(entries)
}

/// Picks the ICO entry closest to the system small-icon size.
fn select_best_ico_entry(entries: &[IcoEntry]) -> Option<&IcoEntry> {
    let desired = unsafe { GetSystemMetrics(SM_CXSMICON) as u32 };
    entries
        .iter()
        .min_by_key(|e| e.width.abs_diff(desired) + e.height.abs_diff(desired))
}

/// Loads the state icon generated by build.rs.
///
/// SAFETY: the ICO data is generated by build.rs and well-formed.
/// CreateIconFromResourceEx returns Err on invalid input.
unsafe fn load_state_icon(icon_type: IconType) -> Option<HICON> {
    let data = icon_bytes(icon_type);
    let entries = parse_ico(data)?;
    let entry = select_best_ico_entry(&entries)?;
    let start = entry.offset as usize;
    let end = start + entry.size as usize;
    let image_data = data.get(start..end)?;
    CreateIconFromResourceEx(
        image_data,
        windows::Win32::Foundation::BOOL(1),
        0x0003_0000,
        entry.width as i32,
        entry.height as i32,
        LR_DEFAULTCOLOR,
    )
    .ok()
}

/// Win32 message-loop thread entry.
///
/// SAFETY: must run on the OS thread created by `std::thread::spawn` so it owns
/// a message queue. This thread creates the hidden window, registers the tray
/// icon, and runs the GetMessageW/DispatchMessageW loop. The HICON and HWND
/// resources are released in WM_DESTROY or cleanup().
unsafe fn tray_thread(tx: Sender<TrayEvent>) -> anyhow::Result<()> {
    // Re-register the icon after explorer.exe restarts.
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

    // Prefer the build.rs state icon; fall back to a system icon on failure.
    let custom_hicon = load_state_icon(IconType::Default);
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
        tracing::error!("Shell_NotifyIconW(NIM_ADD) failed — tray icon may not be visible");
        show_error_message(
            "Syncthing Tray Error",
            "Failed to register tray icon.\nThe daemon is still running, but no icon is visible in the system tray.",
        );
        anyhow::bail!("NIM_ADD failed");
    }

    // Initialize global state.
    let _ = STATE.set(TrayState {
        event_tx: tx,
        hwnd: hwnd.0 as isize,
        taskbar_created_msg,
        current_hicon: custom_hicon.map(|h| h.0 as usize),
    });

    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).into() {
        if msg.message == taskbar_created_msg {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
        DispatchMessageW(&msg);
    }

    Ok(())
}

/// Shows the popup context menu at the right-click position.
///
/// SAFETY: must be called on the Win32 message-loop thread (triggered from
/// window_proc's WM_RBUTTONUP). TrackPopupMenu blocks until the user closes the
/// menu, which is the documented Win32 menu behavior.
unsafe fn show_context_menu(hwnd: HWND) {
    let hmenu = CreatePopupMenu().unwrap_or(HMENU(std::ptr::null_mut()));
    if hmenu.0.is_null() {
        return;
    }

    let running = daemon_running();
    let toggle_text = if running {
        w!("Stop Daemon")
    } else {
        w!("Start Daemon")
    };

    let _ = AppendMenuW(hmenu, MF_STRING, ID_OPEN_TUI as usize, w!("Open TUI"));
    let toggle_flags = if running {
        MF_STRING
    } else {
        MENU_ITEM_FLAGS(MF_STRING.0 | MF_GRAYED.0)
    };
    let _ = AppendMenuW(hmenu, toggle_flags, ID_TOGGLE_DAEMON as usize, toggle_text);
    let _ = AppendMenuW(hmenu, MF_STRING, ID_EXIT as usize, w!("Exit"));

    // SAFETY: SetForegroundWindow requires hwnd to be owned by the current
    // process; hwnd was created by tray_thread.
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
    // Shell convention: post WM_NULL after the menu closes.
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(hmenu);
}

/// Win32 window procedure callback.
///
/// SAFETY: invoked by Windows DispatchMessageW with hwnd/msg/wparam/lparam from
/// the system message queue. The signature must match the `WNDPROC` ABI
/// (`extern "system"`).
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
                WM_LBUTTONUP => {}
                WM_LBUTTONDBLCLK => {
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
            if !CLEANED_UP.load(Ordering::Acquire) {
                let nid = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: TRAY_ICON_ID,
                    ..Default::default()
                };
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                // SAFETY: current_hicon is only released when cleanup() has not run.
                if let Some(s) = STATE.get() {
                    if let Some(ptr) = s.current_hicon {
                        let _ = DestroyIcon(HICON(ptr as *mut _));
                    }
                }
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Cache of the last icon state to avoid redundant refreshes.
static LAST_ICON: std::sync::Mutex<Option<IconType>> = std::sync::Mutex::new(None);
static LAST_TOOLTIP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Tray icon variants mirroring daemon state.
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

/// Updates the tray icon, debounced through the LAST_ICON cache.
///
/// SAFETY: callable from any thread. Shell_NotifyIconW is thread-safe (it
/// talks to the Shell via PostMessage, not SendMessage). The old HICON is
/// released with DestroyIcon here.
pub unsafe fn update_icon(_hwnd: HWND, icon_type: IconType) {
    let Some(state) = STATE.get() else { return };

    if let Ok(guard) = LAST_ICON.lock() {
        if guard.as_ref() == Some(&icon_type) {
            return;
        }
    }

    let new_hicon = match load_state_icon(icon_type) {
        Some(h) => h,
        None => {
            tracing::warn!(
                "Failed to load state icon {:?}, falling back to system icon",
                icon_type
            );
            match LoadIconW(None, icon_type.to_system_icon()) {
                Ok(h) => h,
                Err(_) => return,
            }
        }
    };

    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(state.hwnd as *mut _),
        uID: TRAY_ICON_ID,
        uFlags: NIF_ICON,
        hIcon: new_hicon,
        ..Default::default()
    };
    // SAFETY: NIM_MODIFY is a thread-safe Win32 API call.
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);

    if let Some(old) = STATE.get().and_then(|s| s.current_hicon) {
        let _ = DestroyIcon(HICON(old as *mut _));
    }
    let _ = STATE.get().map(|s| {
        // SAFETY: TrayState lives in the OnceLock; only current_hicon is mutated here.
        let ptr = s as *const TrayState as *mut TrayState;
        (*ptr).current_hicon = Some(new_hicon.0 as usize);
    });

    if let Ok(mut guard) = LAST_ICON.lock() {
        *guard = Some(icon_type);
    }
}

/// Updates the tray tooltip text, debounced through the LAST_TOOLTIP cache.
///
/// SAFETY: callable from any thread. Shell_NotifyIconW(NIM_MODIFY) is thread-safe.
pub unsafe fn update_tooltip(_hwnd: HWND, text: &str) {
    let Some(state) = STATE.get() else { return };

    if let Ok(guard) = LAST_TOOLTIP.lock() {
        if guard.as_deref() == Some(text) {
            return;
        }
    }

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
    // SAFETY: Shell_NotifyIconW is thread-safe.
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);

    if let Ok(mut guard) = LAST_TOOLTIP.lock() {
        *guard = Some(text.to_string());
    }
}

/// Show a Windows tray balloon notification.
///
/// Title truncated to 63 UTF-16 code units; text truncated to 255 UTF-16 code units
/// (NOTIFYICONDATAW limits on Win11).
///
/// SAFETY: Shell_NotifyIconW with NIF_INFO is thread-safe.
/// Content is user-provided notification text; no untrusted input.
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

    // SAFETY: Shell_NotifyIconW is thread-safe.
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}
