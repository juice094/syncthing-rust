# TASK ASSIGNMENT: Agent-Net-1

**Task ID**: NET-001  
**Agent ID**: Agent-Net-1  
**Status**: ASSIGNED  
**Priority**: P0 (Critical)  
**Deadline**: 8 hours from now

---

## Objective

实现完整的 `Transport` trait，使 IrohTransport 可以实际建立 P2P 连接。

## Current State

File: `crates/syncthing-net/src/transport.rs`

Currently:
- `IrohTransport::new()` works
- `connect()` is stub - returns NotImplemented
- `listen()` is stub - returns placeholder listener

## Target State

### 1. Implement `Transport::connect()`

```rust
async fn connect(
    &self,
    addr: &str,
    expected_device: Option<DeviceId>,
) -> Result<Box<dyn BepConnection>>
```

Requirements:
- Parse addr as Iroh Node ID (32-byte hex)
- Connect using Iroh endpoint
- Verify device ID if `expected_device` is Some
- Return `IrohBepConnection` on success

### 2. Implement `Transport::listen()`

```rust
async fn listen(&self, bind_addr: &str) -> Result<Box<dyn ConnectionListener>>
```

Requirements:
- Accept incoming Iroh connections
- Handle connection in background
- Return `IrohConnectionListener`

### 3. Implement `IrohConnectionListener::accept()`

Requirements:
- Wait for incoming connection
- Verify peer identity
- Create and return `IrohBepConnection`

## Deliverables

1. Modified `crates/syncthing-net/src/transport.rs`
2. New tests in `crates/syncthing-net/src/transport.rs` (test module)
3. Integration test helper for two-node setup

## Testing Requirements

Minimum tests:
- `test_transport_connect_local` - connect to self
- `test_transport_listen_accept` - listen and accept
- `test_connection_device_verification` - verify device ID
- `test_two_node_communication` - two transports talk

## Acceptance Criteria

- [ ] `cargo test -p syncthing-net transport` passes
- [ ] New tests ≥ 10
- [ ] `cargo clippy -p syncthing-net` clean
- [ ] No `unwrap()` in production code
- [ ] All public APIs documented

## Verification

Master Agent will run:
```bash
cd C:\Users\22414\Desktop\syncthing-rust-rearch
cargo test -p syncthing-net --test net_acceptance
```

## Notes

- Use Iroh 0.32 API
- See examples at https://iroh.computer/docs
- Connection timeout: 30 seconds default
- Return proper SyncthingError on failure

## Deliver

When complete, write `REPORT.md` in your deliverables directory:

```markdown
# Agent Report: Agent-Net-1

## Task: NET-001
## Status: DELIVERED

## Files Modified
- transport.rs: +200 lines

## Tests Added
- test_transport_connect_local: PASS
- test_transport_listen_accept: PASS
...

## Known Issues
- None / Issue description

## Verification Steps
1. cd to workspace
2. cargo test -p syncthing-net
3. All tests pass
```

---

**WARNING**: This is UNVERIFIED code. Master Agent will test and may request changes.
