#![windows_subsystem = "windows"]
#![allow(clippy::zombie_processes)]

/// syncthing-tray.exe — 薄 wrapper，向后兼容。
///
/// 实际逻辑已合并到 syncthing.exe 中（feature = "tray"）。
/// 此二进制仅查找同目录的 syncthing.exe 并启动它。
use std::path::PathBuf;

fn main() {
    let exe = resolve_syncthing_exe();

    if !exe.exists() {
        eprintln!(
            "syncthing.exe not found at {}. Ensure syncthing.exe is in the same directory.",
            exe.display()
        );
        std::process::exit(1);
    }

    std::process::Command::new(&exe)
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("Failed to start syncthing.exe: {}", e);
            std::process::exit(1);
        });
}

/// 解析 syncthing.exe 的预期路径。
///
/// 优先使用当前可执行文件所在目录；若无法获取则回退到工作目录下的 "syncthing.exe"。
fn resolve_syncthing_exe() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("syncthing.exe")))
        .unwrap_or_else(|| PathBuf::from("syncthing.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_resolve_syncthing_exe_fallback() {
        // 在测试环境中 current_exe() 通常指向测试二进制，其所在目录存在，
        // 因此这里仅验证函数不 panic 且结果以 "syncthing.exe" 结尾。
        let exe = resolve_syncthing_exe();
        assert_eq!(
            exe.file_name(),
            Some(Path::new("syncthing.exe").as_os_str())
        );
    }

    #[test]
    fn test_resolve_syncthing_exe_name_ends_with_exe() {
        let exe = resolve_syncthing_exe();
        let name = exe.file_name().unwrap().to_string_lossy();
        assert!(name.ends_with(".exe"));
    }
}
