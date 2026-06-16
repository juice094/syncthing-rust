//! TCP keepalive 参数按路径类型调优
//!
//! 不同传输路径对空闲连接的容忍度不同：
//! - 直连 TCP：NAT/防火墙通常 5~15 分钟才回收，使用 60s/10s/3 的保守参数。
//! - Relay / Proxy / WebSocket / DERP 等中间节点可能在 60~90s 内掐断空闲连接，
//!   使用 30s/5s/2 的更积极参数。

use syncthing_core::TransportType;
use tokio::net::TcpStream;

/// 为 TCP 流设置 keepalive，参数根据传输类型自动选择。
///
/// 非致命：平台不支持时仅记录 debug 并继续。
pub fn apply_tcp_keepalive(stream: &TcpStream, _transport_type: TransportType) {
    if let Ok(sock) = std::io::Result::Ok(socket2::SockRef::from(stream)) {
        let _ = sock.set_keepalive(true);

        #[cfg(target_os = "linux")]
        {
            use std::time::Duration;
            let transport_type = _transport_type;

            let (time, interval, retries) = match transport_type {
                TransportType::Relay | TransportType::Proxy | TransportType::WebSocket => {
                    (30, 5, 2)
                }
                _ => (60, 10, 3),
            };

            let params = socket2::TcpKeepalive::new()
                .with_time(Duration::from_secs(time))
                .with_interval(Duration::from_secs(interval))
                .with_retries(retries);

            let _ = sock.set_tcp_keepalive(&params);
        }
    }
}
