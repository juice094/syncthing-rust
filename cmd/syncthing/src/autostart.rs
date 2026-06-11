//! Windows 开机自启动注册
//!
//! 通过写入 HKCU\Software\Microsoft\Windows\CurrentVersion\Run 实现。

use std::path::Path;

/// 注册表值名
const REG_VALUE_NAME: &str = "syncthing-rust";

/// 安装开机自启动
pub fn install(config_dir: &Path) -> anyhow::Result<()> {
    let exe = std::env::current_exe().map_err(|e| anyhow::anyhow!("get current exe: {}", e))?;
    let cmd = format!(
        "\"{}\" --config-dir \"{}\"",
        exe.display(),
        config_dir.display()
    );

    let hkey = open_run_key(true)?;
    set_reg_value(hkey, REG_VALUE_NAME, &cmd)?;
    close_key(hkey);
    Ok(())
}

/// 取消开机自启动
pub fn uninstall() -> anyhow::Result<()> {
    let hkey = open_run_key(true)?;
    delete_reg_value(hkey, REG_VALUE_NAME)?;
    close_key(hkey);
    Ok(())
}

// ============================================================
// Windows registry helpers
// ============================================================

#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
#[cfg(windows)]
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, REG_SZ,
};

#[cfg(windows)]
fn open_run_key(_writable: bool) -> anyhow::Result<HKEY> {
    let mut hkey = HKEY(std::ptr::null_mut());
    let path = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    // SAFETY: HKEY_CURRENT_USER 是预定义的根键句柄，path 是常量字符串。
    // RegCreateKeyW 在 HKCU 下创建/打开 Run 子键，不会越权访问。
    unsafe {
        let result = RegCreateKeyW(HKEY_CURRENT_USER, path, &mut hkey);
        if result.0 != 0 {
            anyhow::bail!("RegCreateKeyW failed: error={}", result.0);
        }
    }
    Ok(hkey)
}

#[cfg(windows)]
fn set_reg_value(hkey: HKEY, name: &str, value: &str) -> anyhow::Result<()> {
    let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let value_wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = (value_wide.len() * std::mem::size_of::<u16>()) as u32;
    // SAFETY: name_wide/value_wide 是栈上 Vec 的指针，在函数作用域内有效。
    // from_raw 不获取所有权，slice::from_raw_parts 仅在 RegSetValueExW 调用期间读取。
    unsafe {
        RegSetValueExW(
            hkey,
            windows::core::PCWSTR::from_raw(name_wide.as_ptr()),
            0,
            REG_SZ,
            Some(std::slice::from_raw_parts(
                value_wide.as_ptr() as *const u8,
                byte_len as usize,
            )),
        )
        .ok()
        .map_err(|e| anyhow::anyhow!("RegSetValueExW failed: {}", e))?;
    }
    Ok(())
}

#[cfg(windows)]
fn delete_reg_value(hkey: HKEY, name: &str) -> anyhow::Result<()> {
    let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: name_wide 是栈上 Vec，在函数作用域内有效。from_raw 不获取所有权。
    unsafe {
        let result = RegDeleteValueW(hkey, windows::core::PCWSTR::from_raw(name_wide.as_ptr()));
        if result.0 != 0 {
            if result.0 as u32 == ERROR_FILE_NOT_FOUND.0 {
                return Ok(());
            }
            return Err(anyhow::anyhow!(
                "RegDeleteValueW failed: error={}",
                result.0
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn close_key(hkey: HKEY) {
    // SAFETY: hkey 由 RegCreateKeyW/RegOpenKeyW 返回，必须通过 RegCloseKey 释放。
    // 关闭预定义根键（如 HKCU）是无操作的。
    unsafe {
        let _ = RegCloseKey(hkey);
    }
}

#[cfg(not(windows))]
fn open_run_key(_writable: bool) -> anyhow::Result<HKEY> {
    anyhow::bail!("not supported on this platform")
}
#[cfg(not(windows))]
fn set_reg_value(_hkey: HKEY, _name: &str, _value: &str) -> anyhow::Result<()> {
    anyhow::bail!("not supported on this platform")
}
#[cfg(not(windows))]
fn delete_reg_value(_hkey: HKEY, _name: &str) -> anyhow::Result<()> {
    anyhow::bail!("not supported on this platform")
}
#[cfg(not(windows))]
fn close_key(_hkey: HKEY) {}
