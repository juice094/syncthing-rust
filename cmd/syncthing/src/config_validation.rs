/// 配置验证 — C-UX-4
/// 启动前快速失败，避免配置错误导致运行时静默异常
pub fn validate_config(config: &syncthing_core::types::Config) -> anyhow::Result<()> {
    use std::collections::HashSet;
    use std::net::SocketAddr;
    use syncthing_core::DeviceId;
    use std::str::FromStr;

    // 1. 验证 local_device_id（如果存在）
    if let Some(local_id) = &config.local_device_id {
        let id_str = local_id.to_string();
        if let Err(e) = DeviceId::from_str(&id_str) {
            anyhow::bail!(
                "Invalid local_device_id '{}': {}",
                id_str, e
            );
        }
    }

    // 2. 验证 devices
    let mut seen_ids = HashSet::new();
    for dev in &config.devices {
        let id_str = dev.id.to_string();
        if let Err(e) = DeviceId::from_str(&id_str) {
            anyhow::bail!(
                "Invalid device ID for '{}': {}",
                dev.name.as_deref().unwrap_or("unnamed"), e
            );
        }
        if !seen_ids.insert(dev.id) {
            anyhow::bail!("Duplicate device ID in config: {}", id_str);
        }

        // 验证地址格式
        for addr in &dev.addresses {
            match addr {
                syncthing_core::types::AddressType::Tcp(s) => {
                    if s.parse::<SocketAddr>().is_err() {
                        anyhow::bail!(
                            "Invalid TCP address for device '{}': {}",
                            dev.name.as_deref().unwrap_or("unnamed"), s
                        );
                    }
                }
                syncthing_core::types::AddressType::Relay(s) => {
                    if s.is_empty() {
                        anyhow::bail!(
                            "Empty relay address for device '{}'",
                            dev.name.as_deref().unwrap_or("unnamed")
                        );
                    }
                }
                syncthing_core::types::AddressType::Quic(s) => {
                    if s.parse::<SocketAddr>().is_err() {
                        anyhow::bail!(
                            "Invalid QUIC address for device '{}': {}",
                            dev.name.as_deref().unwrap_or("unnamed"), s
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
            anyhow::bail!("Folder ID cannot be empty");
        }
        let path = std::path::Path::new(&folder.path);
        if !path.exists() {
            anyhow::bail!("Folder path does not exist: {}", folder.path);
        }
        if !path.is_dir() {
            anyhow::bail!("Folder path is not a directory: {}", folder.path);
        }
    }

    // 4. 验证 listen_addr
    if config.listen_addr.parse::<SocketAddr>().is_err() {
        anyhow::bail!("Invalid listen_addr: {}", config.listen_addr);
    }

    Ok(())
}
