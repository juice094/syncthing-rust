use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::{info, warn};

pub fn spawn_port_mapper(
    actual_addr: SocketAddr,
    local_port: u16,
    public_addrs: Arc<Mutex<Vec<String>>>,
    global_discovery: Option<Arc<syncthing_net::GlobalDiscovery>>,
) {
    tokio::spawn(async move {
        let mut port_mapper = syncthing_net::PortMapper::new()
            .with_local_addr(actual_addr);
        match port_mapper.allocate_port(local_port).await {
            Ok(mut mapping) => {
                let mut current_ext = mapping.external_addr();
                let ext_url = format!("tcp://{}", current_ext);
                info!("PortMapper success: {} -> {}", actual_addr, ext_url);
                public_addrs.lock().await.push(ext_url);
                if let Some(ref gd) = global_discovery {
                    gd.trigger_reannounce();
                }

                // 自动续约循环（每 lifetime/2 续约一次）
                loop {
                    let now = std::time::Instant::now();
                    let renew_after = mapping.renew_after();
                    if renew_after <= now {
                        warn!("PortMapper mapping expired before renewal");
                        break;
                    }
                    let sleep_duration = renew_after.duration_since(now);
                    tokio::time::sleep(sleep_duration).await;

                    match port_mapper.allocate_port(local_port).await {
                        Ok(new_mapping) => {
                            let new_ext = new_mapping.external_addr();
                            info!("PortMapper renewed: {} -> {}", actual_addr, new_ext);

                            // 更新 public_addrs：替换旧的外部地址
                            {
                                let mut addrs = public_addrs.lock().await;
                                let old_url = format!("tcp://{}", current_ext);
                                if let Some(pos) = addrs.iter().position(|a| a == &old_url) {
                                    addrs.remove(pos);
                                }
                                addrs.push(format!("tcp://{}", new_ext));
                            }

                            if let Some(ref gd) = global_discovery {
                                gd.trigger_reannounce();
                            }

                            current_ext = new_ext;
                            mapping = new_mapping;
                        }
                        Err(e) => {
                            warn!("PortMapper renewal failed: {}", e);
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                warn!("PortMapper failed (expected if router has no UPnP/NAT-PMP): {}", e);
            }
        }
    });
}

pub fn spawn_stun(
    local_port: u16,
    public_addrs: Arc<Mutex<Vec<String>>>,
    global_discovery: Option<Arc<syncthing_net::GlobalDiscovery>>,
) {
    tokio::spawn(async move {
        let stun = syncthing_net::StunClient::new()
            .with_local_port(local_port);
        match stun.get_public_address().await {
            Ok(pub_addr) => {
                let pub_url = format!("tcp://{}", pub_addr);
                info!("STUN public address: {}", pub_addr);
                public_addrs.lock().await.push(pub_url);
                if let Some(gd) = global_discovery {
                    gd.trigger_reannounce();
                }
            }
            Err(e) => {
                warn!("STUN detection failed (expected behind symmetric NAT/firewall): {}", e);
            }
        }
    });
}
