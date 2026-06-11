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
    DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu,
    HICON, HMENU, IDI_APPLICATION, IDI_ERROR, IDI_INFORMATION, IDI_WARNING, LR_DEFAULTCOLOR,
    MF_STRING, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY,
    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_OVERLAPPED,
};

/// 防止 DestroyIcon double-free：cleanup() 先释放后置 true，WM_DESTROY 检查此标志。
static CLEANED_UP: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    OpenWebUi,
    OpenTui,
    ToggleDaemon,
    Exit,
}

const WM_TRAYICON: u32 = WM_APP + 1;
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
            // SAFETY: custom_hicon 由 CreateIconFromResourceEx 创建，必须通过 DestroyIcon 释放。
            if let Some(ptr) = s.custom_hicon {
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

// 由 build.rs 生成的 ICO 数据（32x32 32bpp ARGB 硬盘图标）
static ICON_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon.ico"));

/// 从 build.rs 生成的 ICO 加载自定义图标。
/// 跳过 6 字节 ICONDIR + 16 字节 ICONDIRENTRY，指向 BITMAPINFOHEADER。
///
/// SAFETY: `ICON_BYTES` 由 build.rs 生成，保证格式为合法的 ICO 文件。
/// 偏移量从 ICONDIRENTRY 固定位置读取（bytes 12-15 = little-endian u32 offset）。
/// 若偏移量越界，`image_data.get(image_offset..)?` 返回 None，安全退出。
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
    // SAFETY: image_data 来自 build.rs 生成的有效 ICO 图像数据，
    // CreateIconFromResourceEx 对无效输入返回 Err。
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
        tracing::error!("Shell_NotifyIconW(NIM_ADD) failed — tray icon may not be visible");
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

/// 在右键点击位置显示弹出上下文菜单。
///
/// SAFETY: 必须在 Win32 消息循环线程上调用（通过 window_proc 的 WM_RBUTTONUP 触发）。
/// TrackPopupMenu 会阻塞当前线程直到用户关闭菜单，这是 Win32 菜单 API 的规范行为。
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
                WM_LBUTTONUP => {
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
            // 若 cleanup() 已先行清理（Exit 路径），跳过重复释放。
            if !CLEANED_UP.load(Ordering::Acquire) {
                let nid = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: hwnd,
                    uID: TRAY_ICON_ID,
                    ..Default::default()
                };
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                // SAFETY: custom_hicon 仅在 cleanup() 未调用时释放。
                if let Some(s) = STATE.get() {
                    if let Some(ptr) = s.custom_hicon {
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
/// HICON 句柄由 LoadIconW（系统图标，无需释放）或 STATE.custom_hicon（自定义图标，托管于 OnceLock）提供。
pub unsafe fn update_icon(_hwnd: HWND, icon_type: IconType) {
    let Some(state) = STATE.get() else { return };

    // 防抖：状态未变时跳过
    if let Ok(guard) = LAST_ICON.lock() {
        if guard.as_ref() == Some(&icon_type) {
            return;
        }
    }

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
    // SAFETY: NIM_MODIFY 是线程安全的 Win32 API 调用。
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);

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
