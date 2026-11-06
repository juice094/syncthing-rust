//! Windows autostart registration via the HKCU Run key.

use std::path::Path;

#[cfg(windows)]
/// Registry value name under the Run key.
const REG_VALUE_NAME: &str = "syncthing-rust";

#[cfg(windows)]
/// Registers the binary to start on login.
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

#[cfg(windows)]
/// Removes the autostart entry.
pub fn uninstall() -> anyhow::Result<()> {
    let hkey = open_run_key(true)?;
    delete_reg_value(hkey, REG_VALUE_NAME)?;
    close_key(hkey);
    Ok(())
}

#[cfg(not(windows))]
#[allow(dead_code)]
/// Install autostart (unsupported on this platform).
pub fn install(_config_dir: &Path) -> anyhow::Result<()> {
    anyhow::bail!("autostart is only supported on Windows")
}

#[cfg(not(windows))]
#[allow(dead_code)]
/// Remove autostart (unsupported on this platform).
pub fn uninstall() -> anyhow::Result<()> {
    anyhow::bail!("autostart is only supported on Windows")
}

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
    // SAFETY: HKEY_CURRENT_USER is a predefined root key handle and path is a
    // constant string. RegCreateKeyW only creates/opens the Run subkey under
    // HKCU, with no privilege escalation.
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
    // SAFETY: name_wide/value_wide point at stack Vecs valid for the function
    // scope. from_raw borrows without taking ownership; slice::from_raw_parts
    // is only read for the duration of the RegSetValueExW call.
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
    // SAFETY: name_wide is a stack Vec valid for the function scope; from_raw borrows without taking ownership.
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
    // SAFETY: hkey was returned by RegCreateKeyW/RegOpenKeyW and must be
    // released with RegCloseKey. Closing a predefined root key (e.g. HKCU) is
    // a no-op.
    unsafe {
        let _ = RegCloseKey(hkey);
    }
}
