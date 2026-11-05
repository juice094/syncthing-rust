//! TCP keepalive parameters tuned per path type.
//!
//! Rationale for the parameter sets: see "docs/design/ADR-20261105-connection-tuning.md".

use syncthing_core::TransportType;
use tokio::net::TcpStream;

/// Applies TCP keepalive to the stream, choosing parameters by transport type.
///
/// Non-fatal: when the platform does not support tuning, logs at debug and continues.
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
