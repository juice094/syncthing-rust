/// 配置验证 — C-UX-4
/// 启动前快速失败，避免配置错误导致运行时静默异常
pub fn validate_config(config: &syncthing_core::types::Config) -> anyhow::Result<()> {
    validate_config_internal(config, true)
}

/// TUI 模式下的轻量验证 — 不检查 folder path 存在性。
/// TUI 是配置编辑器，用户可能正在创建尚未存在的文件夹。
pub fn validate_config_non_blocking(config: &syncthing_core::types::Config) -> anyhow::Result<()> {
    validate_config_internal(config, false)
}

fn validate_config_internal(
    config: &syncthing_core::types::Config,
    check_folder_paths: bool,
) -> anyhow::Result<()> {
    use std::collections::HashSet;
    use std::net::SocketAddr;
    use std::str::FromStr;
    use syncthing_core::DeviceId;

    // 1. 验证 local_device_id（如果存在）
    if let Some(local_id) = &config.local_device_id {
        let id_str = local_id.to_string();
        if let Err(e) = DeviceId::from_str(&id_str) {
            anyhow::bail!(
                "Invalid local_device_id '{}': {}\n\
                 修复: 删除 config.json 中的 local_device_id 字段，启动时自动生成",
                id_str,
                e
            );
        }
    }

    // 2. 验证 devices
    let mut seen_ids = HashSet::new();
    for dev in &config.devices {
        let id_str = dev.id.to_string();
        if let Err(e) = DeviceId::from_str(&id_str) {
            anyhow::bail!(
                "Invalid device ID for '{}': {}\n\
                 修复: Device ID 格式为 XXXXXXX-XXXXXXX-XXXXXXX-XXXXXXX-XXXXXXX-XXXXXXX-XXXXXXX-XXXXXXX\n\
                 可通过 syncthing-cli show-id 查看对侧设备 ID",
                dev.name.as_deref().unwrap_or("unnamed"),
                e
            );
        }
        if !seen_ids.insert(dev.id) {
            anyhow::bail!(
                "Duplicate device ID in config: {}\n\
                 修复: 删除重复的 device 条目，每个设备 ID 只能出现一次",
                id_str
            );
        }

        // 验证地址格式
        for addr in &dev.addresses {
            match addr {
                syncthing_core::types::AddressType::Tcp(s) => {
                    if s.parse::<SocketAddr>().is_err() {
                        anyhow::bail!(
                            "Invalid TCP address for device '{}': {}\n\
                             修复: 地址格式应为 host:port，如 192.168.1.100:22001 或 tailscale IP",
                            dev.name.as_deref().unwrap_or("unnamed"),
                            s
                        );
                    }
                }
                syncthing_core::types::AddressType::Relay(s) => {
                    if s.is_empty() {
                        anyhow::bail!(
                            "Empty relay address for device '{}'\n\
                             修复: 提供有效的 relay URL 或删除该地址",
                            dev.name.as_deref().unwrap_or("unnamed")
                        );
                    }
                }
                syncthing_core::types::AddressType::Quic(s) => {
                    if s.parse::<SocketAddr>().is_err() {
                        anyhow::bail!(
                            "Invalid QUIC address for device '{}': {}\n\
                             修复: 地址格式应为 host:port",
                            dev.name.as_deref().unwrap_or("unnamed"),
                            s
                        );
                    }
                }
                syncthing_core::types::AddressType::Dynamic => {}
            }
        }
    }

    // 3. 验证 folders
    for folder in &config.folders {
        if folder.id.is_empty() {
            anyhow::bail!("Folder ID cannot be empty\n修复: 为文件夹指定唯一标识符");
        }
        if check_folder_paths {
            let path = std::path::Path::new(&folder.path);
            if !path.exists() {
                anyhow::bail!(
                    "Folder path does not exist: {}\n\
                     修复: 创建该目录或修改 config.json 中的 path 字段",
                    folder.path
                );
            }
            if !path.is_dir() {
                anyhow::bail!(
                    "Folder path is not a directory: {}\n\
                     修复: 将 path 指向一个目录而非文件",
                    folder.path
                );
            }
        }
    }

    // 4. 验证 listen_addr
    if config.listen_addr.parse::<SocketAddr>().is_err() {
        anyhow::bail!(
            "Invalid listen_addr: {}\n\
             修复: 格式应为 host:port，如 0.0.0.0:22001",
            config.listen_addr
        );
    }

    Ok(())
}
