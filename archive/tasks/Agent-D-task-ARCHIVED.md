# Agent-D Task: Network Layer

## ⚠️ CRITICAL RULES

1. **DO NOT MODIFY** `syncthing-core` crate - it is READ ONLY
2. **ALL DELIVERIES** must be marked `UNVERIFIED`
3. **MUST IMPLEMENT** `Transport`, `Discovery`, `ConnectionListener` traits
4. **Handle all error cases** gracefully

## Task Overview

Implement network layer in `crates/syncthing-net/`.

## Deliverables

### 1. transport.rs - Transport Implementation

```rust
//! Network transport layer
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{
    traits::{BepConnection, ConnectionListener, Transport},
    DeviceId, Result,
};
use async_trait::async_trait;

/// QUIC transport implementation
pub struct QuicTransport {
    endpoint: quinn::Endpoint,
}

impl QuicTransport {
    /// Create new transport
    pub async fn new(bind_addr: &str) -> Result<Self> {
        // UNVERIFIED
    }
}

#[async_trait]
impl Transport for QuicTransport {
    // Implement trait methods
}
```

### 2. discovery.rs - Device Discovery

```rust
//! Device discovery
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{
    traits::{AnnouncementHandle, Discovery},
    DeviceId, Result,
};

/// Local multicast discovery
pub struct LocalDiscovery {
    // fields
}

impl LocalDiscovery {
    /// Create new local discovery
    pub fn new() -> Result<Self> {
        // UNVERIFIED
    }
}

#[async_trait]
impl Discovery for LocalDiscovery {
    // Implement trait methods
}
```

### 3. nat/ - NAT Traversal

```rust
//! NAT traversal modules
//! 
//! ⚠️ STATUS: UNVERIFIED

pub mod upnp;
pub mod natpmp;
pub mod stun;

// upnp.rs
pub async fn map_port_upnp(
    external_port: u16,
    internal_port: u16,
) -> Result<PortMapping> {
    // UNVERIFIED
}
```

### 4. relay.rs - Relay Connection

```rust
//! Relay server connection
//! 
//! ⚠️ STATUS: UNVERIFIED

/// Connect via relay server
pub async fn connect_relay(
    relay_addr: &str,
    device_id: DeviceId,
) -> Result<Box<dyn BepConnection>> {
    // UNVERIFIED
}
```

## Requirements

1. Support both QUIC and TCP (QUIC preferred)
2. Implement local discovery (multicast)
3. NAT traversal: UPnP, NAT-PMP
4. Relay fallback support

## Testing

- Mock transport for testing
- Test connection establishment
- Test discovery protocol
