//! Generate a proper syncthing config.json for stress testing.
//!
//! Usage:
//!   gen_test_config --output-dir ./node \
//!       --device-id LOCAL_ID --peer-id PEER_ID --peer-addr "tcp://100.127.13.26:22001" \
//!       --sync-path "/tmp/sync" --listen "0.0.0.0:22001"

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "gen_test_config")]
struct Args {
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    device_id: String,
    #[arg(long)]
    peer_id: String,
    #[arg(long)]
    peer_addr: String,
    #[arg(long)]
    sync_path: String,
    #[arg(long)]
    listen: String,
    #[arg(long, default_value = "stress-test-node")]
    device_name: String,
}

fn main() {
    let args = Args::parse();

    let device_id = args
        .device_id
        .parse::<syncthing_core::DeviceId>()
        .expect("invalid device id");
    let peer_id = args
        .peer_id
        .parse::<syncthing_core::DeviceId>()
        .expect("invalid peer device id");

    let mut config = syncthing_core::types::Config::new();
    config.listen_addr = args.listen;
    config.device_name = args.device_name;
    config.local_device_id = Some(device_id);

    config.devices.push(syncthing_core::types::Device {
        id: peer_id,
        name: Some("peer".to_string()),
        addresses: vec![syncthing_core::types::AddressType::Tcp(
            args.peer_addr.replace("tcp://", ""),
        )],
        paused: false,
        introducer: false,
    });

    let mut folder = syncthing_core::types::Folder::new("stress-test", &args.sync_path);
    folder.label = Some("Stress Test Folder".to_string());
    folder.devices = vec![peer_id];
    folder.rescan_interval_secs = 10;
    config.folders.push(folder);

    config.gui.enabled = false;
    config.options.relays_enabled = false;

    let config_path = args.output_dir.join("config.json");
    std::fs::create_dir_all(&args.output_dir).expect("create output dir");
    let json = serde_json::to_string_pretty(&config).expect("serialize config");
    std::fs::write(&config_path, json).expect("write config");

    println!("Config written to {}", config_path.display());
}
