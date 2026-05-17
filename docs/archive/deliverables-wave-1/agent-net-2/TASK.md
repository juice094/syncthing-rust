# TASK ASSIGNMENT: Agent-Net-2

**Task ID**: NET-002  
**Agent ID**: Agent-Net-2  
**Status**: ASSIGNED  
**Priority**: P0 (Critical)  
**Deadline**: 8 hours from now  
**Dependency**: NET-001 (wait for transport)

---

## Objective

集成 BEP 协议消息与 Iroh 双向流，实现完整的 BEP 消息交换。

## Current State

File: `crates/syncthing-net/src/connection.rs`

Currently:
- `IrohBepConnection` has stub methods
- Message sending uses mpsc channel (not Iroh)
- No actual stream integration

## Target State

### 1. Integrate Iroh Bi-directional Streams

```rust
// In IrohBepConnection::new()
let (mut send_stream, mut recv_stream) = connection.open_bi().await?;
```

### 2. Implement Message Serialization

```rust
async fn send_message(&mut self, msg: BepMessage) -> Result<()> {
    // Serialize to bytes
    // Write to send_stream
    // Handle flush
}

async fn recv_message(&mut self) -> Result<Option<BepMessage>> {
    // Read from recv_stream
    // Deserialize
    // Return message
}
```

### 3. Implement All BepConnection Methods

- `send_index()` - Send Index message
- `send_index_update()` - Send IndexUpdate
- `request_block()` - Send Request, wait for Response
- `recv_message()` - Receive any message type

### 4. Message Types to Support

- Hello (during handshake)
- Index
- IndexUpdate
- Request
- Response
- DownloadProgress
- Ping/Pong (keepalive)

## Deliverables

1. Modified `crates/syncthing-net/src/connection.rs`
2. New `crates/syncthing-net/src/message_stream.rs` (if needed)
3. Tests for each message type

## Testing Requirements

Minimum tests:
- `test_send_index_roundtrip` - send and receive Index
- `test_request_block` - request and receive block data
- `test_ping_pong` - keepalive
- `test_message_ordering` - order preserved
- `test_concurrent_messages` - multiple messages

## Acceptance Criteria

- [ ] All BEP message types can be sent/received
- [ ] Message order preserved
- [ ] Handles connection loss gracefully
- [ ] Tests ≥ 10
- [ ] No unwrap() in production code

## Verification

Master Agent will run:
```bash
cargo test -p syncthing-net connection
cargo test --test net_acceptance net_002
```

## Notes

- Use `connection.open_bi()` for bidirectional stream
- Each message needs length prefix for framing
- Consider message pipelining
- Handle partial reads/writes

## Serialization Format

```rust
// Length-prefixed message
[4 bytes: message length][N bytes: protobuf message]
```

Use `tokio::io::AsyncReadExt` / `AsyncWriteExt` for stream operations.

## Deliver

Write `REPORT.md` in your deliverables directory when complete.

---

**WARNING**: This is UNVERIFIED code. Master Agent will test and may request changes.
