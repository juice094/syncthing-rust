//! CLI 初始化向导 — C-UX-1
//!
//! 交互式生成 config.json，降低手动编辑 JSON 的配置门槛。

use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;
use syncthing_core::types::{AddressType, Config, Device, Folder, FolderType};

fn prompt(message: &str, default: Option<&str>) -> String {
    print!("{}", message);
    if let Some(d) = default {
        print!(" [{}]", d);
    }
    print!(": ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        default.unwrap_or("").to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn run_wizard(config_dir: &PathBuf) -> anyhow::Result<()> {
    println!("========================================");
    println!("  Syncthing-Rust 初始化向导");
    println!("========================================");
    println!();

    // 1. 设备名称
    let device_name = prompt("1. 本设备名称", Some("syncthing-rust"));

    // 2. 同步文件夹路径
    let folder_path = loop {
        let path = prompt("2. 同步文件夹路径（绝对路径）", None);
        if path.is_empty() {
            println!("   错误：路径不能为空\n");
            continue;
        }
        let p = std::path::Path::new(&path);
        if !p.exists() {
            println!("   警告：路径不存在，将自动创建\n");
            if let Err(e) = std::fs::create_dir_all(p) {
                println!("   创建失败：{}，请重新输入\n", e);
                continue;
            }
        } else if !p.is_dir() {
            println!("   错误：路径不是目录，请重新输入\n");
            continue;
        }
        break path;
    };

    // 3. 对侧设备 ID
    let peer_id = loop {
        let id = prompt("3. 对侧设备 ID（如 XXXXXXX-XXXXXXX-...）", None);
        if id.is_empty() {
            println!("   错误：设备 ID 不能为空\n");
            continue;
        }
        if let Err(e) = syncthing_core::DeviceId::from_str(&id) {
            println!("   错误：无效的 Device ID — {}\n", e);
            continue;
        }
        break id;
    };

    // 4. 对侧地址
    let peer_addr = loop {
        let addr = prompt(
            "4. 对侧地址（如 192.168.1.100:22001 或 tailscale IP）",
            None,
        );
        if addr.is_empty() {
            println!("   错误：地址不能为空\n");
            continue;
        }
        if addr.parse::<std::net::SocketAddr>().is_err() {
            println!("   错误：地址格式不正确，应为 host:port\n");
            continue;
        }
        break addr;
    };

    // 5. 生成本地设备 ID
    let local_device_id = syncthing_core::DeviceId::random();
    println!();
    println!("生成本地设备 ID: {}", local_device_id);

    // 6. 组装配置
    let config = Config {
        version: 1,
        listen_addr: "0.0.0.0:22001".to_string(),
        device_name,
        folders: vec![Folder {
            id: "default".to_string(),
            path: folder_path,
            label: None,
            folder_type: FolderType::SendReceive,
            paused: false,
            rescan_interval_secs: 10,
            devices: vec![syncthing_core::DeviceId::from_str(&peer_id).unwrap()],
            ignore_patterns: vec![],
            versioning: None,
        }],
        devices: vec![Device {
            id: syncthing_core::DeviceId::from_str(&peer_id).unwrap(),
            name: Some("peer".to_string()),
            addresses: vec![AddressType::Tcp(peer_addr)],
            paused: false,
            introducer: false,
        }],
        local_device_id: Some(local_device_id),
        gui: syncthing_core::types::GuiConfig {
            enabled: true,
            address: "0.0.0.0:8385".to_string(),
            api_key: random_api_key(),
        },
        options: syncthing_core::types::Options {
            listen_addresses: vec![],
            global_announce_enabled: false,
            local_announce_enabled: false,
            relays_enabled: false,
        },
    };

    // 7. 保存
    let config_path = config_dir.join("config.json");
    std::fs::create_dir_all(config_dir)?;
    let content = serde_json::to_string_pretty(&config)?;
    std::fs::write(&config_path, content)?;

    println!();
    println!("========================================");
    println!("  配置已保存到: {}", config_path.display());
    println!("========================================");
    println!();
    println!("下一步:");
    println!("  1. 在对侧设备上运行相同的向导，输入本侧设备 ID");
    println!(
        "  2. 启动服务: syncthing run --config-dir {}",
        config_dir.display()
    );
    println!();

    Ok(())
}

fn random_api_key() -> String {
    use rand::Rng;
    let chars: Vec<char> = (b'a'..=b'z')
        .chain(b'0'..=b'9')
        .map(|c| c as char)
        .collect();
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| chars[rng.gen_range(0..chars.len())])
        .collect()
}
