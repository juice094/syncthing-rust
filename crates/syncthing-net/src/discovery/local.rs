//! Local Discovery (UDP broadcast / multicast)
//!
//! Compatible with syncthing-go local discovery protocol.
//! See `docs/design/NETWORK_DISCOVERY_DESIGN.md` for full protocol details.

use std::net::SocketAddr;
use std::time::Duration;
use syncthing_core::{DeviceId, Result, SyncthingError};
use tokio::net::UdpSocket;

/// Magic number for local discovery (same as BEP Hello).
pub const LOCAL_DISCOVERY_MAGIC: u32 = 0x2EA7D90B;

/// UDP port for local discovery.
pub const LOCAL_DISCOVERY_PORT: u16 = 21027;

/// Broadcast / multicast interval.
pub const LOCAL_DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);

/// Cache entry TTL (3 × interval).
pub const LOCAL_DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// Protobuf Announce message
// ---------------------------------------------------------------------------

/// Local discovery Announce message.
///
/// Wire format (protobuf):
/// ```protobuf
/// message Announce {
///     bytes id = 1;
///     repeated string addresses = 2;
///     int64 instance_id = 3;
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Announce {
    pub id: Vec<u8>,
    pub addresses: Vec<String>,
    pub instance_id: i64,
}

impl Announce {
    /// Encode to protobuf bytes.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();

        // field 1: id (bytes, tag = 1 << 3 | 2 = 10)
        if !self.id.is_empty() {
            buf.push(0x0a);
            buf.push(self.id.len() as u8);
            buf.extend_from_slice(&self.id);
        }

        // field 2: addresses (string, repeated, tag = 2 << 3 | 2 = 18)
        for addr in &self.addresses {
            if !addr.is_empty() {
                buf.push(0x12);
                buf.push(addr.len() as u8);
                buf.extend_from_slice(addr.as_bytes());
            }
        }

        // field 3: instance_id (int64, tag = 3 << 3 | 0 = 24)
        if self.instance_id != 0 {
            buf.push(0x18);
            put_varint(&mut buf, self.instance_id as u64);
        }

        Ok(buf)
    }

    /// Decode from protobuf bytes.
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut announce = Self {
            id: Vec::new(),
            addresses: Vec::new(),
            instance_id: 0,
        };

        let mut i = 0;
        while i < data.len() {
            let tag = data[i];
            i += 1;
            let field_num = (tag >> 3) as u32;
            let wire_type = tag & 0x7;

            match (field_num, wire_type) {
                (1, 2) => {
                    // bytes
                    if i >= data.len() {
                        break;
                    }
                    let len = data[i] as usize;
                    i += 1;
                    if i + len > data.len() {
                        break;
                    }
                    announce.id = data[i..i + len].to_vec();
                    i += len;
                }
                (2, 2) => {
                    // string
                    if i >= data.len() {
                        break;
                    }
                    let len = data[i] as usize;
                    i += 1;
                    if i + len > data.len() {
                        break;
                    }
                    let s = std::str::from_utf8(&data[i..i + len])
                        .map_err(|e| SyncthingError::protocol(format!("invalid utf8: {}", e)))?;
                    announce.addresses.push(s.to_string());
                    i += len;
                }
                (3, 0) => {
                    // int64 varint
                    if i >= data.len() {
                        break;
                    }
                    let (val, consumed) = decode_varint(&data[i..]);
                    announce.instance_id = val as i64;
                    i += consumed;
                }
                _ => {
                    // Unknown field — for our simple 3-field message we can stop.
                    break;
                }
            }
        }

        Ok(announce)
    }
}

fn put_varint(buf: &mut Vec<u8>, mut val: u64) {
    while val >= 0x80 {
        buf.push((val as u8) | 0x80);
        val >>= 7;
    }
    buf.push(val as u8);
}

fn decode_varint(data: &[u8]) -> (u64, usize) {
    let mut val = 0u64;
    let mut i = 0;
    loop {
        if i >= data.len() {
            break;
        }
        let b = data[i];
        val |= ((b & 0x7f) as u64) << (7 * i);
        i += 1;
        if b & 0x80 == 0 {
            break;
        }
    }
    (val, i)
}

// ---------------------------------------------------------------------------
// LocalDiscovery service
// ---------------------------------------------------------------------------

/// UDP broadcast / multicast discovery service.
pub struct LocalDiscovery {
    device_id: DeviceId,
    instance_id: u64,
    announce_addrs: Vec<String>,
    port: u16,
}

impl LocalDiscovery {
    /// Create a new local discovery service.
    pub fn new(device_id: DeviceId, announce_addrs: Vec<String>) -> Self {
        Self {
            device_id,
            instance_id: rand::random(),
            announce_addrs,
            port: LOCAL_DISCOVERY_PORT,
        }
    }

    /// Override the port (useful for testing).
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Send a single broadcast announcement.
    pub async fn broadcast(&self) -> Result<()> {
        let socket = UdpSocket::bind("0.0.0.0:0").await
            .map_err(|e| SyncthingError::network(format!("bind failed: {}", e)))?;
        socket.set_broadcast(true)
            .map_err(|e| SyncthingError::network(format!("set_broadcast failed: {}", e)))?;

        let announce = Announce {
            id: self.device_id.as_bytes().to_vec(),
            addresses: self.announce_addrs.clone(),
            instance_id: self.instance_id as i64,
        };

        let payload = announce.encode()?;
        let mut packet = Vec::with_capacity(4 + payload.len());
        packet.extend_from_slice(&LOCAL_DISCOVERY_MAGIC.to_be_bytes());
        packet.extend_from_slice(&payload);

        let broadcast_addr = SocketAddr::from(([255, 255, 255, 255], self.port));
        match socket.send_to(&packet, broadcast_addr).await {
            Ok(n) => tracing::debug!("Sent {} bytes broadcast to {}", n, broadcast_addr),
            Err(e) => tracing::warn!("Broadcast send failed: {}", e),
        }

        Ok(())
    }

    /// Listen for broadcast announcements.
    /// Returns the first valid Announce received (for testing).
    pub async fn listen_once(&self) -> Result<(Announce, SocketAddr)> {
        let bind_addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        let socket = UdpSocket::bind(bind_addr).await
            .map_err(|e| SyncthingError::network(format!("bind failed: {}", e)))?;

        let mut buf = vec![0u8; 65536];
        let (len, addr) = socket.recv_from(&mut buf).await
            .map_err(|e| SyncthingError::network(format!("recv failed: {}", e)))?;

        if len < 4 {
            return Err(SyncthingError::protocol("packet too short"));
        }

        let magic = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if magic != LOCAL_DISCOVERY_MAGIC {
            return Err(SyncthingError::protocol(format!("magic mismatch: {:08x}", magic)));
        }

        let announce = Announce::decode(&buf[4..len])?;
        Ok((announce, addr))
    }

    /// Run the discovery service (broadcast sender + listener).
    ///
    /// When a valid Announce from a *different* device is received, a
    /// `DiscoveryEvent::DeviceDiscovered` is sent via `event_tx`.
    pub async fn run(
        &self,
        event_tx: tokio::sync::mpsc::Sender<super::events::DiscoveryEvent>,
    ) -> Result<()> {
        let broadcast_interval = tokio::time::interval(LOCAL_DISCOVERY_INTERVAL);
        tokio::pin!(broadcast_interval);

        let bind_addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        let socket = UdpSocket::bind(bind_addr).await
            .map_err(|e| SyncthingError::network(format!("listen bind failed: {}", e)))?;

        let mut buf = vec![0u8; 65536];

        loop {
            tokio::select! {
                _ = broadcast_interval.tick() => {
                    if let Err(e) = self.broadcast().await {
                        tracing::warn!("Broadcast error: {}", e);
                    }
                }
                result = socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, addr)) => {
                            if len < 4 { continue; }
                            let magic = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                            if magic != LOCAL_DISCOVERY_MAGIC { continue; }
                            match Announce::decode(&buf[4..len]) {
                                Ok(announce) => {
                                    match DeviceId::from_bytes(&announce.id) {
                                        Ok(device_id) if device_id != self.device_id => {
                                            tracing::info!(
                                                "Local discovery: {} at {:?} (from {})",
                                                device_id, announce.addresses, addr
                                            );
                                            let _ = event_tx.send(
                                                super::events::DiscoveryEvent::DeviceDiscovered {
                                                    device_id,
                                                    addresses: announce.addresses,
                                                    source: super::events::DiscoverySource::Local,
                                                }
                                            ).await;
                                        }
                                        Ok(_) => {
                                            // Own announce, ignore
                                        }
                                        Err(e) => {
                                            tracing::debug!("Bad device id in announce: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!("Bad announce from {}: {}", addr, e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Receive error: {}", e);
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_announce_encode_decode_roundtrip() {
        let original = Announce {
            id: vec![0x01, 0x02, 0x03, 0x04],
            addresses: vec!["tcp://192.168.1.10:22000".to_string()],
            instance_id: 42,
        };

        let encoded = original.encode().unwrap();
        let decoded = Announce::decode(&encoded).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_announce_empty() {
        let original = Announce {
            id: vec![],
            addresses: vec![],
            instance_id: 0,
        };

        let encoded = original.encode().unwrap();
        assert!(encoded.is_empty());
    }

    #[test]
    fn test_announce_multiple_addresses() {
        let original = Announce {
            id: vec![0xab; 32],
            addresses: vec![
                "tcp://192.168.1.10:22000".to_string(),
                "tcp://[::1]:22000".to_string(),
            ],
            instance_id: 123_456_789,
        };

        let encoded = original.encode().unwrap();
        let decoded = Announce::decode(&encoded).unwrap();

        assert_eq!(original.id, decoded.id);
        assert_eq!(original.addresses, decoded.addresses);
        assert_eq!(original.instance_id, decoded.instance_id);
    }

    /// UDP broadcast roundtrip test.
    /// Uses a random high port to avoid conflicts.
    #[tokio::test]
    async fn test_udp_broadcast_roundtrip() {
        use std::str::FromStr;
        let device_id = syncthing_core::DeviceId::from_str(
            "YTKWHNG-OT27ZGH-6VVBRIJ-OHOUNWT-DYLJ2NR-TCXUXHI-QDUQR2U-OPLCBQG",
        ).unwrap();
        let addrs = vec!["tcp://127.0.0.1:22001".to_string()];
        let port = 54_321u16;

        let discovery = LocalDiscovery::new(device_id.clone(), addrs.clone()).with_port(port);

        // Spawn listener
        let listen_handle = tokio::spawn(async move {
            discovery.listen_once().await
        });

        // Give listener time to bind
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Send broadcast
        let sender = LocalDiscovery::new(device_id.clone(), addrs.clone()).with_port(port);
        sender.broadcast().await.unwrap();

        // Wait for reception (with timeout)
        let result = tokio::time::timeout(Duration::from_secs(5), listen_handle).await;
        let (received, from_addr) = result
            .expect("listener timed out")
            .expect("listener panicked")
            .expect("listen_once failed");

        assert_eq!(received.id, device_id.as_bytes().to_vec());
        assert_eq!(received.addresses, addrs);
        assert!(from_addr.ip().is_loopback() || from_addr.ip().is_unspecified() || from_addr.ip().is_ipv4());
    }
}
