//! Windows 系统托盘（Win32 API），参照 tray-icon / systray 标准实现。
//!
//! TrayApp::spawn() 在独立线程创建隐藏窗口 + 托盘图标 + 消息循环。
//! 通过 mpsc::Receiver 向主线程发送托盘事件。

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

/// 防止 DestroyIcon double-free：cleanup() 先释放后置 true，WM_DESTROY 检查此标志。
static CLEANED_UP: AtomicBool = AtomicBool::new(false);

/// 当前 daemon 运行状态，供菜单渲染时读取。
/// 由主线程在 daemon 启动/停止时更新。
static DAEMON_RUNNING: AtomicBool = AtomicBool::new(false);

/// 设置 daemon 运行状态（主线程调用）。
pub fn set_daemon_running(running: bool) {
    DAEMON_RUNNING.store(running, Ordering::Release);
}

/// 读取 daemon 是否正在运行（托盘线程调用）。
pub fn daemon_running() -> bool {
    DAEMON_RUNNING.load(Ordering::Acquire)
}

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

/// 线程安全的状态存储（替代 static mut）
static STATE: OnceLock<TrayState> = OnceLock::new();

struct TrayState {
    event_tx: Sender<TrayEvent>,
    hwnd: isize,
    /// Kept for TaskbarCreated message comparison; static analysis cannot see the use.
    #[allow(dead_code)]
    taskbar_created_msg: u32,
    /// 当前显示图标的 HICON（任意状态），以 usize 存储是因为原始指针非 Send/Sync。
    current_hicon: Option<usize>,
}

/// Windows 错误消息框（用于无控制台场景）。
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

/// 在独立 Win32 线程上启动托盘图标和消息循环。
///
/// 返回 `(Receiver<TrayEvent>, JoinHandle)` 供主线程消费事件并在退出时 join 线程。
/// SAFETY: 内部调用 `tray_thread` 通过 `std::thread::spawn` 启动，保证仅调用一次。
pub fn spawn() -> (Receiver<TrayEvent>, std::thread::JoinHandle<()>) {
    let (tx, rx) = channel::<TrayEvent>();
    let handle = std::thread::spawn(move || {
        // SAFETY: tray_thread 必须在 Win32 线程上运行以拥有消息循环窗口。
        // 此 closure 在 `std::thread::spawn` 创建的 OS 线程上执行，满足要求。
        unsafe {
            if let Err(e) = tray_thread(tx) {
                tracing::error!("Tray thread error: {}", e);
            }
        }
    });
    (rx, handle)
}

/// 主动清理托盘图标（供 Exit 前调用）。
///
/// SAFETY: 仅在主线程的 Exit 路径调用，此时托盘图标已通过 NIM_ADD 注册。
/// Shell_NotifyIconW 和 DestroyIcon 均为线程安全的系统调用。
/// 调用后 PostMessage(WM_CLOSE) 通知托盘线程退出消息循环。
pub fn cleanup() {
    if let Some(s) = STATE.get() {
        // SAFETY: STATE 已通过 OnceLock 初始化，hwnd/custom_hicon 均有效。
        // NIM_DELETE 是线程安全的 Win32 API。
        unsafe {
            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: HWND(s.hwnd as *mut _),
                uID: TRAY_ICON_ID,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            // SAFETY: current_hicon 由 CreateIconFromResourceEx 创建，必须通过 DestroyIcon 释放。
            if let Some(ptr) = s.current_hicon {
                let _ = DestroyIcon(HICON(ptr as *mut _));
            }
            // 设置标志防止 WM_DESTROY 再次释放 icon
            CLEANED_UP.store(true, Ordering::Release);
            // 通知托盘线程退出：PostMessage(WM_CLOSE) → DefWindowProc → DestroyWindow → WM_DESTROY → PostQuitMessage
            let _ = PostMessageW(HWND(s.hwnd as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// 返回托盘窗口句柄（供 update_icon / update_tooltip 使用）
pub fn tray_hwnd() -> HWND {
    HWND(STATE.get().map(|s| s.hwnd).unwrap_or(0) as *mut _)
}

// 由 build.rs 生成的多尺寸 ICO 数据（16/24/32/48/256）
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

/// 从 ICO 数据中选择最接近系统小图标尺寸的 entry。
fn select_best_ico_entry(entries: &[IcoEntry]) -> Option<&IcoEntry> {
    let desired = unsafe { GetSystemMetrics(SM_CXSMICON) as u32 };
    entries
        .iter()
        .min_by_key(|e| e.width.abs_diff(desired) + e.height.abs_diff(desired))
}

/// 从 build.rs 生成的多尺寸 ICO 加载指定状态图标。
///
/// SAFETY: ICO 数据由 build.rs 生成，格式合法。CreateIconFromResourceEx 对无效输入返回 Err。
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

/// Win32 消息循环线程入口。
///
/// SAFETY: 必须在 `std::thread::spawn` 创建的拥有消息队列的 OS 线程上调用。
/// 此线程负责：创建隐藏窗口、注册托盘图标、运行 GetMessageW/DispatchMessageW 循环。
/// HICON 和 HWND 资源需在 WM_DESTROY 或 cleanup() 中释放。
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

    // 优先使用 build.rs 生成的状态图标；失败时回退到系统图标
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

    // 初始化全局状态
    let _ = STATE.set(TrayState {
        event_tx: tx,
        hwnd: hwnd.0 as isize,
        taskbar_created_msg,
        current_hicon: custom_hicon.map(|h| h.0 as usize),
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

/// 在右键点击位置显示弹出上下文菜单。
///
/// SAFETY: 必须在 Win32 消息循环线程上调用（通过 window_proc 的 WM_RBUTTONUP 触发）。
/// TrackPopupMenu 会阻塞当前线程直到用户关闭菜单，这是 Win32 菜单 API 的规范行为。
unsafe fn show_context_menu(hwnd: HWND) {
    let hmenu = CreatePopupMenu().unwrap_or(HMENU(std::ptr::null_mut()));
    if hmenu.0.is_null() {
        return;
    }

    // 操作项（无图标、无标题，保持菜单极简）
    // daemon 运行时显示 "Stop Daemon"，停止后显示灰色的 "Start Daemon"（暂不支持托盘内启动）。
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

    // SAFETY: SetForegroundWindow 需要 hwnd 为当前进程拥有的窗口句柄。
    // hwnd 由 tray_thread 创建，属于当前进程。
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

/// Win32 窗口过程回调。
///
/// SAFETY: 由 Windows DispatchMessageW 调用，hwnd/msg/wparam/lparam 均来自系统消息队列。
/// 函数签名必须匹配 `WNDPROC` 的 ABI 约定（`extern "system"`）。
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
                // 单击托盘图标不执行操作；双击打开 TUI。避免双击时被识别为两次单击
                // 而产生重复 TUI 实例。
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
            // 若 cleanup() 已先行清理（Exit 路径），跳过重复释放。
            if !CLEANED_UP.load(Ordering::Acquire) {
                let nid = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: TRAY_ICON_ID,
                    ..Default::default()
                };
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                // SAFETY: current_hicon 仅在 cleanup() 未调用时释放。
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

/// 上一次图标状态缓存，避免无意义刷新
static LAST_ICON: std::sync::Mutex<Option<IconType>> = std::sync::Mutex::new(None);
static LAST_TOOLTIP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

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

/// 更新托盘图标。通过 LAST_ICON 缓存防抖。
///
/// SAFETY: 可从任意线程调用。Shell_NotifyIconW 是线程安全的，
/// 内部通过 PostMessage 而非 SendMessage 与 Shell 通信。
/// 旧 HICON 会在这里被 DestroyIcon 释放。
pub unsafe fn update_icon(_hwnd: HWND, icon_type: IconType) {
    let Some(state) = STATE.get() else { return };

    // 防抖：状态未变时跳过
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
    // SAFETY: NIM_MODIFY 是线程安全的 Win32 API 调用。
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);

    // 释放旧图标并缓存新图标句柄
    if let Some(old) = STATE.get().and_then(|s| s.current_hicon) {
        let _ = DestroyIcon(HICON(old as *mut _));
    }
    let _ = STATE.get().map(|s| {
        // SAFETY: TrayState 存于 OnceLock，此处仅修改 current_hicon 字段。
        let ptr = s as *const TrayState as *mut TrayState;
        (*ptr).current_hicon = Some(new_hicon.0 as usize);
    });

    if let Ok(mut guard) = LAST_ICON.lock() {
        *guard = Some(icon_type);
    }
}

/// 更新托盘 tooltip 文本。通过 LAST_TOOLTIP 缓存防抖。
///
/// SAFETY: 可从任意线程调用。Shell_NotifyIconW(NIM_MODIFY) 是线程安全的。
pub unsafe fn update_tooltip(_hwnd: HWND, text: &str) {
    let Some(state) = STATE.get() else { return };

    // 防抖：tooltip 未变时跳过
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
    // SAFETY: Shell_NotifyIconW 线程安全。
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

    // SAFETY: Shell_NotifyIconW 线程安全。
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}
