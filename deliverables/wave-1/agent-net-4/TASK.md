# TASK ASSIGNMENT: Agent-Net-4

**Task ID**: NET-004  
**Agent ID**: Agent-Net-4  
**Status**: ASSIGNED  
**Priority**: P1 (High)  
**Deadline**: 6 hours from now  
**Dependency**: NET-001

---

## Objective

实现连接管理器，支持连接池、保活和自动重连。

## New File

Create: `crates/syncthing-net/src/manager.rs`

## Target State

### 1. ConnectionPool

```rust
pub struct ConnectionPool {
    connections: Arc<RwLock<HashMap<DeviceId, PooledConnection>>>,
    max_connections: usize,
    keepalive_interval: Duration,
}

struct PooledConnection {
    connection: Box<dyn BepConnection>,
    last_used: Instant,
    use_count: u64,
}
```

### 2. ConnectionManager

```rust
pub struct ConnectionManager {
    pool: ConnectionPool,
    transport: Arc<IrohTransport>,
    config: ConnectionConfig,
}

impl ConnectionManager {
    /// Get or create connection to device
    pub async fn get_connection(&self, device: &DeviceId) 
        -> Result<Box<dyn BepConnection>>;
    
    /// Close connection and remove from pool
    pub async fn close_connection(&self, device: &DeviceId) -> Result<()>;
    
    /// Start background tasks (keepalive, cleanup)
    pub async fn start_maintenance(&self);
}
```

### 3. Features

- **Connection Pooling**: Reuse connections to same device
- **Keepalive**: Ping/Pong to keep connections alive
- **Auto-reconnect**: Reconnect on connection loss
- **Concurrent Limit**: Max N concurrent connections
- **Idle Timeout**: Close unused connections after timeout

## Deliverables

1. New `crates/syncthing-net/src/manager.rs`
2. Tests in `src/manager.rs` (test module)
3. Update `lib.rs` to export

## Testing Requirements

Minimum tests:
- `test_pool_reuse` - connection reused
- `test_pool_limit` - max connections enforced
- `test_keepalive` - ping/pong works
- `test_auto_reconnect` - reconnects on drop
- `test_idle_timeout` - idle connection closed
- `test_concurrent_access` - thread-safe

## Acceptance Criteria

- [ ] Connection pooling works
- [ ] Keepalive prevents timeout
- [ ] Auto-reconnect on failure
- [ ] Concurrent limit enforced
- [ ] Tests ≥ 8

## Verification

Master Agent will run:
```bash
cargo test -p syncthing-net manager
cargo test --test net_acceptance net_004
```

## Notes

Keepalive interval: 30 seconds default
Idle timeout: 5 minutes default
Max connections: 100 default

Use `tokio::time::interval` for periodic tasks.

## Deliver

Write `REPORT.md` in your deliverables directory when complete.

---

**WARNING**: This is UNVERIFIED code. Master Agent will test and may request changes.
