//! 单实例锁 — C-UX-5
//!
//! 防止同一配置目录下启动多个 syncthing-rust 进程，避免 device ID 冲突、
//! 端口占用和连接状态混乱。

use std::path::Path;
use sysinfo::{ProcessRefreshKind, RefreshKind, System};

const PID_FILE_NAME: &str = "syncthing.pid";

/// 尝试获取单实例锁。
///
/// 检查 config_dir/syncthing.pid：
/// - 若不存在 → 写入当前 PID，返回 Ok
/// - 若存在且对应进程仍在运行 → 返回 Err，提示用户
/// - 若存在但对应进程已消失 → 覆盖写入当前 PID，返回 Ok
pub fn acquire(config_dir: &Path) -> Result<(), String> {
    let pid_file = config_dir.join(PID_FILE_NAME);

    if pid_file.exists() {
        let pid_str = std::fs::read_to_string(&pid_file).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            let s = System::new_with_specifics(
                RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
            );
            if s.process(sysinfo::Pid::from(pid as usize)).is_some() {
                return Err(format!(
                    "Error: Another syncthing-rust instance is already running (PID {}).\n       If you want to run multiple instances, use --config-dir to separate configurations.",
                    pid
                ));
            }
        }
    }

    std::fs::create_dir_all(config_dir).map_err(|e| {
        format!(
            "Failed to create config dir '{}': {}",
            config_dir.display(),
            e
        )
    })?;

    std::fs::write(&pid_file, format!("{}\n", std::process::id()))
        .map_err(|e| format!("Failed to write pid file '{}': {}", pid_file.display(), e))?;

    Ok(())
}

/// 释放单实例锁（删除 pid 文件）。
pub fn release(config_dir: &Path) {
    let _ = std::fs::remove_file(config_dir.join(PID_FILE_NAME));
}
