# TASK ASSIGNMENT: Agent-Net-3

**Task ID**: NET-003  
**Agent ID**: Agent-Net-3  
**Status**: ASSIGNED  
**Priority**: P0 (Critical)  
**Deadline**: 6 hours from now

---

## Objective

实现完整的设备发现机制，包括 Iroh DHT 和本地多播。

## Current State

File: `crates/syncthing-net/src/discovery.rs`

Currently:
- Local cache only
- No Iroh DHT integration
- `announce()` only stores locally

## Target State

### 1. Integrate Iroh DHT Discovery

```rust
pub struct IrohDiscovery {
    local_cache: Arc<RwLock<HashMap<DeviceId, Vec<String>>>>,
    iroh_discovery: Option<Box<dyn iroh::discovery::Discovery>>,
}
```

### 2. Implement DHT-based Lookup

```rust
async fn lookup(&self, device: &DeviceId) -> Result<Vec<String>> {
    // 1. Check local cache first
    // 2. If not found, query Iroh DHT
    // 3. Cache and return results
}
```

### 3. Implement DHT Announcement

```rust
async fn announce(&self, device: &DeviceId, addresses: Vec<String>) -> Result<()> {
    // 1. Store locally
    // 2. Publish to Iroh DHT
    // 3. Return success
}
```

### 4. Add Local Multicast (Optional but recommended)

For local network discovery:
- UDP multicast on 239.31.31.31:21027
- Broadcast device info periodically
- Listen for other devices

## Deliverables

1. Modified `crates/syncthing-net/src/discovery.rs`
2. Optional: `src/discovery/multicast.rs` for local discovery
3. Tests for discovery scenarios

## Testing Requirements

Minimum tests:
- `test_dht_lookup` - lookup via DHT (mock)
- `test_dht_announce` - announce to DHT (mock)
- `test_cache_refresh` - cache behavior
- `test_multicast_local` - local discovery (if implemented)
- `test_periodic_announce` - auto-refresh

## Acceptance Criteria

- [ ] Can announce device to DHT
- [ ] Can lookup device from DHT
- [ ] Local cache works
- [ ] Periodic announcement works
- [ ] Tests ≥ 8

## Verification

Master Agent will run:
```bash
cargo test -p syncthing-net discovery
cargo test --test net_acceptance net_003
```

## Notes

Iroh Discovery API:
```rust
// Example Iroh DHT usage
let discovery = iroh::discovery::DhtDiscovery::new()?;
discovery.publish(node_id, &addresses).await?;
let found = discovery.resolve(node_id).await?;
```

See Iroh docs for actual API.

## Deliver

Write `REPORT.md` in your deliverables directory when complete.

---

**WARNING**: This is UNVERIFIED code. Master Agent will test and may request changes.
