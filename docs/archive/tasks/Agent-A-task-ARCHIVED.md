# Agent-A Task: BEP Protocol Implementation

## ⚠️ CRITICAL RULES

1. **DO NOT MODIFY** `syncthing-core` crate - it is READ ONLY
2. **ALL DELIVERIES** must be marked `UNVERIFIED`
3. **MUST IMPLEMENT** all traits from `syncthing-core::traits`
4. **Unit tests required** for all public functions

## Task Overview

Implement the Block Exchange Protocol (BEP) in `crates/bep-protocol/`.

## Deliverables

### 1. messages.rs - Protocol Buffer Messages

Define Protocol Buffer messages for:
- `Hello` - Initial handshake
- `ClusterConfig` - Folder/device configuration exchange
- `Index` - Full file index
- `IndexUpdate` - Delta index update
- `Request` - Block request
- `Response` - Block data response
- `DownloadProgress` - Progress updates

Use `prost` for protobuf encoding.

### 2. codec.rs - Message Encoding/Decoding

```rust
//! Message codec for BEP
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{Result, SyncthingError};
use bytes::{Bytes, BytesMut};

/// Message codec
pub struct BepCodec {
    max_message_size: usize,
}

impl BepCodec {
    /// Create new codec
    pub fn new(max_size: usize) -> Self {
        Self { max_message_size: max_size }
    }
    
    /// Encode a message to bytes
    pub fn encode(&mut self, msg: BepMessage) -> Result<Bytes> {
        // UNVERIFIED implementation
    }
    
    /// Decode bytes to message
    pub fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<BepMessage>> {
        // UNVERIFIED implementation
    }
}
```

### 3. handshake.rs - TLS Handshake

Implement TLS 1.2/1.3 handshake with certificate validation:

```rust
//! TLS Handshake for BEP
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{DeviceId, Result};

/// Perform TLS handshake and return authenticated device ID
pub async fn handshake(
    stream: tokio::net::TcpStream,
    local_cert: &rustls::Certificate,
    local_key: &rustls::PrivateKey,
) -> Result<(tokio_rustls::server::TlsStream<tokio::net::TcpStream>, DeviceId)> {
    // UNVERIFIED implementation
}
```

### 4. connection.rs - BEP Connection

Implement `BepConnection` trait from `syncthing-core`:

```rust
//! BEP Connection implementation
//! 
//! ⚠️ STATUS: UNVERIFIED

use syncthing_core::{
    traits::{BepConnection, BepMessage},
    BlockHash, DeviceId, FileInfo, FolderId, Result,
};
use async_trait::async_trait;

pub struct BepConnectionImpl {
    // fields
}

#[async_trait]
impl BepConnection for BepConnectionImpl {
    // ALL methods must be implemented
}
```

## Requirements

1. Support BEP version 1
2. Support LZ4 compression (optional for MVP)
3. Support message pipelining
4. Handle connection errors gracefully

## Testing

Create unit tests in each module covering:
- Message encode/decode roundtrip
- Handshake success/failure
- Connection message exchange

## KNOWN_ISSUES.md Template

Create `crates/bep-protocol/KNOWN_ISSUES.md`:

```markdown
# Known Issues - bep-protocol

## UNVERIFIED Implementation
- All code is preliminary and untested
- Integration with actual Syncthing devices not verified

## TODO
- [ ] LZ4 compression support
- [ ] Message pipelining optimization
- [ ] Connection keepalive
```
