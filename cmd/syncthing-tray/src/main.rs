#![windows_subsystem = "windows"]
#![allow(clippy::zombie_processes)]

/// syncthing-tray.exe — 薄 wrapper，向后兼容。
///
/// 实际逻辑已合并到 syncthing.exe 中（feature = "tray"）。
/// 此二进制仅查找同目录的 syncthing.exe 并启动它。
fn main() {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("syncthing.exe")))
        .unwrap_or_else(|| std::path::PathBuf::from("syncthing.exe"));

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
