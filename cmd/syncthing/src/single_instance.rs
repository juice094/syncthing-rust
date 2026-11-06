//! Single-instance lock (C-UX-5).
//!
//! Prevents multiple syncthing-rust processes from sharing one config
//! directory, which would cause device ID conflicts, port collisions, and
//! inconsistent connection state.

use std::path::Path;
use sysinfo::{ProcessRefreshKind, RefreshKind, System};

const PID_FILE_NAME: &str = "syncthing.pid";

/// Acquires the single-instance lock.
///
/// Checks `config_dir/syncthing.pid`:
/// - missing → writes the current PID and returns Ok;
/// - present with a live process → returns Err telling the user;
/// - present but the process is gone → overwrites with the current PID and returns Ok.
pub fn acquire(config_dir: &Path) -> Result<(), String> {
    let pid_file = config_dir.join(PID_FILE_NAME);

    if pid_file.exists() {
        let pid_str = std::fs::read_to_string(&pid_file).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            let s = System::new_with_specifics(
                RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
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

/// Releases the lock by deleting the pid file.
pub fn release(config_dir: &Path) {
    let _ = std::fs::remove_file(config_dir.join(PID_FILE_NAME));
}

/// Checks whether the pid file points at a live syncthing-rust instance.
///
/// Returns `Some(pid)` when another instance is running; `None` when there is
/// no instance or the pid file is stale.
pub fn running_instance_pid(config_dir: &Path) -> Option<u32> {
    let pid_file = config_dir.join(PID_FILE_NAME);
    if !pid_file.exists() {
        return None;
    }
    let pid_str = std::fs::read_to_string(&pid_file).unwrap_or_default();
    let pid = pid_str.trim().parse::<u32>().ok()?;
    if pid == std::process::id() {
        return None;
    }
    if is_process_alive(pid) {
        Some(pid)
    } else {
        None
    }
}

/// Checks whether the process with the given PID is still alive.
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let s = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );
    s.process(sysinfo::Pid::from(pid as usize)).is_some()
}
