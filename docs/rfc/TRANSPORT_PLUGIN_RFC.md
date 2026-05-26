---
type: rfc
status: draft
project: syncthing-rust
tags: [rfc, design]
---

# RFC: Transport Plugin Architecture

> **Status**: Draft  
> **Author**: syncthing-rust team  
> **Date**: 2026-05-18  
> **Target Version**: v0.3.0  
> **Tracking**: Phase 3 of v0.2.9-rc1 → v0.3.0 plan

---

## 1. Background & Motivation

### 1.1 Problem Statement

syncthing-rust currently operates with a **hardcoded TCP-first** networking model. While the codebase contains the infrastructure for multi-transport (the `Transport` trait, `TransportRegistry`, and `AddressType` enum with URL scheme support), the critical path for dialing and listening is **not scheme-aware**.

This creates the following production blockers:

| ID | Blocker | Impact |
|----|---------|--------|
| B-1 | Campus/enterprise firewalls block TCP 22001 outbound | Personal users in universities and corporate networks cannot sync with external nodes ([KNOWN_ISSUES.md §8](../../KNOWN_ISSUES.md)) |
| B-2 | Government extranets only allow ports 80/443/22/UDP 443 | TCP 22001 is effectively impossible to approve; syncthing-rust is **not deployable** in Chinese government/enterprise contexts without overlay network workarounds |
| B-3 | No config-driven transport selection | Adding a new transport (e.g., Tailscale, WireGuard, WebSocket) requires modifying `daemon_runner.rs` and recompiling |
| B-4 | Port 22001 hardcoded in 6+ locations | Inconsistent defaults; port collisions with Go Syncthing (22000) were "solved" by picking a different magic number instead of making it configurable |

### 1.2 Current Workarounds (and why they are insufficient)

- **Tailscale/Headscale overlay**: Works but requires external infrastructure and manual address configuration. The daemon itself has no knowledge of the overlay.
- **Relay v1 fallback**: Available but not automatically prioritized; relay servers may not exist in air-gapped environments.

### 1.3 Goal

Enable **configuration-driven, runtime-switchable transport plugins** so that syncthing-rust can operate over arbitrary underlying byte pipes (TCP, WebSocket, Tailscale SOCKS, WireGuard, Unix socket) without code changes or recompilation.

---

## 2. Current Architecture Analysis

### 2.1 TransportRegistry: Exists but Unused in Dialing

```rust
// crates/syncthing-net/src/transport/mod.rs:24-66
pub struct TransportRegistry {
    transports: Vec<Arc<dyn Transport>>,
}

impl TransportRegistry {
    pub fn get(&self, scheme: &str) -> Option<Arc<dyn Transport>> { ... }
    pub fn default_transport(&self) -> Option<Arc<dyn Transport>> { self.transports.first().cloned() }
}
```

**Finding**: `get(scheme)` is implemented but **never called** in the dial path. The only consumer is `ConnectionManager::start_listening()`, which uses `default_transport()` — always returning the first registered transport (TCP).

### 2.2 ParallelDialer: Single Connector, Scheme-Blind

```rust
// crates/syncthing-net/src/dialer/mod.rs:165-178
pub struct ParallelDialer {
    scores: DashMap<SocketAddr, AddressScore>,
    device_scores: DashMap<DeviceId, Vec<AddressScore>>,
    local_device_id: DeviceId,
    device_name: String,
    connector: RwLock<Arc<dyn DialConnector>>,   // <-- SINGLE connector
    relay_connector: Option<Arc<dyn RelayDialConnector>>,
}
```

**Finding**: `ParallelDialer` holds exactly one `DialConnector`. The `with_tcp_connector()` constructor hardcodes `TcpBepConnector`. There is no mechanism to select a different connector based on the address scheme.

**Critical**: The `dial()` method receives `Vec<SocketAddr>`. The `SocketAddr` type does **not** carry scheme information. By the time addresses reach the dialer, the original `tcp://`, `relay://`, or future `ws://` prefix has been stripped.

### 2.3 Address Pipeline: Scheme Stripped Early

```rust
// AddressType preserves scheme in config
AddressType::Tcp("host:port")   // serialized as "tcp://host:port"
AddressType::Quic("host:port")  // serialized as "quic://host:port"
AddressType::Relay("url")       // serialized as "relay://url"

// But ParallelDialer::dial() receives:
Vec<SocketAddr>  // scheme is GONE
```

**Finding**: The address-to-`SocketAddr` conversion happens upstream (in `ConnectionManager` or device address resolution), losing scheme metadata before the dialer sees the address.

### 2.4 Port 22001 Hardcoding Audit

| File | Line | Context |
|------|------|---------|
| `crates/syncthing-core/src/types/mod.rs` | 337 | `default_listen()` |
| `cmd/syncthing/src/main.rs` | 41, 52, 120, 509, 542, 558, 583 | CLI args, test assertions, wizard defaults |
| `cmd/syncthing/src/init_wizard.rs` | 96 | Wizard default prompt |
| `cmd/syncthing/src/bin/gen_test_config.rs` | 5-6 | Test config generator help text |

**Finding**: Port 22001 is a magic number scattered across CLI parsing, config defaults, test assertions, and user-facing prompts. No single constant exists.

### 2.5 Transport Registration: Manual, Compile-Time

```rust
// cmd/syncthing/src/tui/daemon_runner.rs:151-164
let mut transport_registry = syncthing_net::transport::TransportRegistry::new();
transport_registry.register(Arc::new(syncthing_net::transport::RawTcpTransport::new()));
transport_registry.register(Arc::new(syncthing_net::derp::DerpTransport::new(device_id)));
if let Some(proxy) = syncthing_net::transport::proxy::ProxiedTransport::from_env() {
    transport_registry.register(Arc::new(proxy));
}
```

**Finding**: Transports are registered in source code. Adding WebSocket, QUIC, or a custom Tailscale transport requires editing `daemon_runner.rs` and recompiling.

---

## 3. Design

### 3.1 Principle: Scheme as First-Class Routing Key

The core insight is that **scheme (`tcp`, `quic`, `ws`, `relay`, `derp`, `unix`) is the natural routing key for transport selection**. The current system almost has this right — `AddressType` carries scheme, `Transport::scheme()` returns it, and `TransportRegistry::get()` looks it up — but the routing table is bypassed in the dial path.

We will fix this by making scheme flow through the entire address → dial → connect pipeline.

### 3.2 Design Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Transport Plugin Architecture                        │
├─────────────────────────────────────────────────────────────────────────────┤
│  Config Layer          ┌─────────────┐                                      │
│  (config.json)         │ transports  │ ──► ["tcp", "websocket", "derp"]    │
│                        └──────┬──────┘                                      │
│                               │                                              │
│  Registration Layer    ┌──────▼──────┐                                      │
│  (daemon_runner.rs)    │ Transport   │ ◄─── scheme → Transport mapping     │
│                        │ Registry    │      (dynamic, config-driven)       │
│                        └──────┬──────┘                                      │
│                               │                                              │
│  Address Pipeline      ┌──────▼──────┐                                      │
│                        │  Address    │ ──► (scheme, SocketAddr) pairs      │
│                        │  Routing    │      scheme preserved through dial   │
│                        └──────┬──────┘                                      │
│                               │                                              │
│  Dial Layer            ┌──────▼──────┐                                      │
│                        │ Parallel    │ ◄─── per-scheme connector lookup    │
│                        │ Dialer      │      via TransportRegistry            │
│                        └──────┬──────┘                                      │
│                               │                                              │
│  Transport Layer       ┌──────▼──────┐                                      │
│                        │  Scheme-    │ ──► TcpTransport / WsTransport      │
│                        │  specific   │      / DerpTransport / etc.         │
│                        │  Connector  │                                      │
│                        └─────────────┘                                      │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 3.3 Batch 3.1: Centralized Port Constant

**Goal**: Eliminate magic number 22001.

**New file**: `crates/syncthing-core/src/constants.rs`

```rust
//! Centralized constants for syncthing-rust.
//!
//! All hardcoded defaults must live here. No magic numbers in CLI parsing,
//! config defaults, or test assertions.

/// Default BEP listen port.
///
/// Historical note: Go Syncthing uses 22000. Rust implementation uses 22001
/// to avoid port collision when running side-by-side for interoperability testing.
pub const DEFAULT_BEP_PORT: u16 = 22001;

/// Default REST API port.
pub const DEFAULT_API_PORT: u16 = 8385;

/// Default listen address for BEP (all interfaces).
pub const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:22001";

/// Default API listen address (localhost only for security).
pub const DEFAULT_API_ADDR: &str = "127.0.0.1:8385";

/// Maximum concurrent dial attempts per device.
pub const MAX_PARALLEL_DIALS: usize = 3;
```

**Replacement sites**:
1. `crates/syncthing-core/src/types/mod.rs:336-338` — `default_listen()`
2. `cmd/syncthing/src/main.rs:41,52` — CLI `#[arg(default_value)]`
3. `cmd/syncthing/src/init_wizard.rs:96` — wizard prompt default
4. All test assertions that assert `== "0.0.0.0:22001"`

**Backward compatibility**: Existing `config.json` files with explicit `"listen_addr": "0.0.0.0:22001"` continue to work. The constant only affects default construction.

### 3.4 Batch 3.2: Scheme-Preserving Address Pipeline

**Goal**: Ensure `ParallelDialer::dial()` knows the scheme of each address.

**Problem**: Currently `dial()` takes `Vec<SocketAddr>`. `SocketAddr` is `(IP, port)` with no scheme.

**Solution**: Introduce a lightweight wrapper that pairs scheme with address.

```rust
// crates/syncthing-core/src/types/connection.rs

/// A resolved network address with its transport scheme preserved.
///
/// This type flows through the address pipeline from config → discovery →
/// connection manager → dialer, ensuring the dialer can select the correct
/// transport implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedAddress {
    /// Transport scheme: "tcp", "quic", "ws", "relay", "derp", "unix", etc.
    pub scheme: String,
    /// Resolved socket address (IP + port).
    pub addr: SocketAddr,
    /// Optional: raw URL for transports that need it (e.g., relay://host:port?id=...)
    pub raw: Option<String>,
}

impl ResolvedAddress {
    pub fn new(scheme: impl Into<String>, addr: SocketAddr) -> Self {
        Self {
            scheme: scheme.into(),
            addr,
            raw: None,
        }
    }

    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }
}
```

**Pipeline changes**:

1. **Config → Address resolution**: When parsing `AddressType::Tcp("host:port")`, produce `ResolvedAddress { scheme: "tcp", addr: resolved_socket_addr }`.

2. **Discovery**: Global discovery returns URLs like `tcp://203.0.113.5:22001`. Parse into `ResolvedAddress { scheme: "tcp", addr, raw: Some(url) }`.

3. **ConnectionManager**: Store `Vec<ResolvedAddress>` per device instead of `Vec<SocketAddr>`.

4. **ParallelDialer**: Change signature:

```rust
// BEFORE
pub async fn dial(
    &self,
    device_id: DeviceId,
    addresses: Vec<SocketAddr>,
    relay_urls: Vec<String>,
    ...
) -> Result<Arc<BepConnection>, SyncthingError>

// AFTER
pub async fn dial(
    &self,
    device_id: DeviceId,
    addresses: Vec<ResolvedAddress>,
    ...
) -> Result<Arc<BepConnection>, SyncthingError>
```

**Note**: `relay_urls` are absorbed into `ResolvedAddress` with `scheme: "relay"` and `raw: Some(url)`.

### 3.5 Batch 3.3: Per-Scheme DialConnector

**Goal**: Make `ParallelDialer` look up the correct connector per address scheme.

**New abstraction**: `SchemeDialConnector` — a connector that wraps a `Transport` and adapts it to the `DialConnector` trait.

```rust
// crates/syncthing-net/src/dialer/mod.rs

/// Adapts a generic Transport to the DialConnector interface.
///
/// This bridges the gap between the low-level Transport trait (raw byte pipe)
/// and the high-level DialConnector (TLS + BEP handshake).
pub struct TransportDialConnector {
    transport: Arc<dyn Transport>,
}

#[async_trait]
impl DialConnector for TransportDialConnector {
    async fn connect(
        &self,
        addr: SocketAddr,
        device_id: DeviceId,
        local_device_id: DeviceId,
        device_name: &str,
        tls_config: &Arc<SyncthingTlsConfig>,
    ) -> Result<Arc<BepConnection>, SyncthingError> {
        // 1. Establish raw byte pipe via Transport
        let pipe = self.transport.dial(addr).await?;

        // 2. TLS handshake
        // 3. BEP Hello
        // ... existing connect_bep logic extracted here
        connect_bep_over_pipe(pipe, addr, device_id, local_device_id, device_name, tls_config).await
    }
}
```

**ParallelDialer refactoring**:

```rust
pub struct ParallelDialer {
    scores: DashMap<ResolvedAddress, AddressScore>,  // key changes from SocketAddr to ResolvedAddress
    device_scores: DashMap<DeviceId, Vec<AddressScore>>,
    local_device_id: DeviceId,
    device_name: String,
    // REMOVED: connector: RwLock<Arc<dyn DialConnector>>
    // ADDED: scheme → connector mapping
    connectors: DashMap<String, Arc<dyn DialConnector>>,
    // Reference to TransportRegistry for on-demand connector creation
    transport_registry: RwLock<Option<Arc<TransportRegistry>>>,
}
```

**Connector resolution in `dial()`**:

```rust
for resolved in &top_addresses {
    let connector = self.connectors
        .get(&resolved.scheme)
        .or_else(|| {
            // Lazily create connector from TransportRegistry if not cached
            let registry = self.transport_registry.read();
            registry.as_ref()?.get(&resolved.scheme).map(|transport| {
                let connector: Arc<dyn DialConnector> =
                    Arc::new(TransportDialConnector::new(transport));
                self.connectors.insert(resolved.scheme.clone(), Arc::clone(&connector));
                connector
            })
        })
        .ok_or_else(|| SyncthingError::connection(
            format!("no transport registered for scheme: {}", resolved.scheme)
        ))?;

    // Spawn dial task with this connector...
}
```

**Relay handling**: Relay URLs become `ResolvedAddress { scheme: "relay", addr: relay_socket_addr, raw: Some(relay_url) }`. The relay connector is either:
- Pre-registered in `connectors` as a special-case `Arc<dyn RelayDialConnector>` adapted to `DialConnector`, or
- Handled by a `RelayTransport` implementing `Transport` with `scheme() -> "relay"`.

**Recommended**: Promote relay to a full `Transport` implementation (`RelayTransport`) with `scheme() -> "relay"`, eliminating the special-case `relay_connector` field.

### 3.6 Batch 3.4: Config-Driven Transport Registration

**Goal**: Allow `config.json` to specify which transports to enable.

**Config extension**:

```json
{
  "transports": {
    "enabled": ["tcp", "websocket", "derp"],
    "tcp": {
      "bind_interface": "0.0.0.0",
      "port": 22001
    },
    "websocket": {
      "bind_interface": "0.0.0.0",
      "port": 22002,
      "path": "/bep"
    },
    "derp": {
      "region": "home",
      "urls": ["https://derp.example.com"]
    }
  }
}
```

**Registration logic in `daemon_runner.rs`**:

```rust
let transport_config = config.transports.clone().unwrap_or_default();
let mut registry = TransportRegistry::new();

for scheme in &transport_config.enabled {
    match scheme.as_str() {
        "tcp" => registry.register(Arc::new(RawTcpTransport::new())),
        "websocket" => {
            let cfg = transport_config.websocket.unwrap_or_default();
            registry.register(Arc::new(WebSocketTransport::new(cfg)));
        }
        "derp" => {
            let cfg = transport_config.derp.clone().unwrap_or_default();
            registry.register(Arc::new(DerpTransport::new(device_id, cfg)));
        }
        "proxy" => {
            if let Some(proxy) = ProxiedTransport::from_env() {
                registry.register(Arc::new(proxy));
            }
        }
        other => {
            warn!("Unknown transport scheme '{}' in config, skipping", other);
        }
    }
}

// If no transports explicitly configured, default to TCP only
if registry.schemes().is_empty() {
    registry.register(Arc::new(RawTcpTransport::new()));
}

manager.set_transport_registry(Arc::new(registry));
```

**Default behavior**: If `transports.enabled` is absent (backward compatibility), default to `["tcp"]`.

### 3.7 Batch 3.5: ConnectionManager Transport Selection for Listening

**Current**: `ConnectionManager::start_listening()` uses `default_transport()`.

**New**: Start a listener for **each enabled transport**.

```rust
// In ConnectionManager::start_listening()
let registry = self.transport_registry.read();
let Some(registry) = registry.as_ref() else { /* fallback to TCP */ };

let mut listen_addrs = Vec::new();

for scheme in registry.schemes() {
    let transport = registry.get(scheme).expect("scheme exists");

    // Determine bind address for this transport
    let bind_addr = self.config.transport_bind_addr(scheme)
        .unwrap_or_else(|| self.config.listen_addr.parse().expect("valid addr"));

    let listener = BepTransportListener::start(
        transport,
        &bind_addr.to_string(),
        handle.clone(),
        self.local_device_id,
        self.config.device_name.clone(),
        Arc::clone(&self.tls_config),
    ).await?;

    listen_addrs.push((scheme.to_string(), listener.local_addr()?));
}
```

**Implication**: A node can simultaneously listen on TCP 22001, WebSocket 22002, and DERP. Remote peers can connect via whichever transport their network allows.

---

## 4. API Surface Changes

### 4.1 Public API Changes

| Component | Change | Breaking? |
|-----------|--------|-----------|
| `ParallelDialer::dial()` | `Vec<SocketAddr>` → `Vec<ResolvedAddress>` | Yes (internal) |
| `ParallelDialer::with_tcp_connector()` | Deprecated; use `with_transport_registry()` | Yes (internal) |
| `Config` | Add `transports: Option<TransportConfig>` | No (optional field) |
| `syncthing_core::constants` | New module; public constants | No (additive) |
| `AddressScore::address` | `SocketAddr` → `ResolvedAddress` | Yes (internal) |

### 4.2 Config Compatibility

- **Old configs** without `transports` field: default to TCP-only (identical behavior).
- **New configs** with `transports.enabled`: register only listed transports.
- **Mixed environments**: A node with `["tcp", "websocket"]` can talk to a node with `["tcp"]` over TCP. WebSocket is only used if both sides enable it.

---

## 5. Implementation Batches

| Batch | Content | Estimated Effort | Files |
|-------|---------|------------------|-------|
| 3.1 | Centralized constants (`syncthing-core/src/constants.rs`) | 2h | New file + 6 replacements |
| 3.2 | `ResolvedAddress` type + address pipeline | 4h | `types/connection.rs`, `manager/mod.rs`, `discovery/` |
| 3.3 | `TransportDialConnector` + `ParallelDialer` per-scheme routing | 1d | `dialer/mod.rs`, `dialer/tests.rs` |
| 3.4 | Config-driven registration in `daemon_runner.rs` | 4h | `daemon_runner.rs`, `types/mod.rs` (Config) |
| 3.5 | Multi-transport listening in `ConnectionManager` | 4h | `manager/mod.rs` |
| 3.6 | WebSocket transport activation | 4h | `transport/websocket.rs` (exists but dormant) |
| 3.7 | Unit + integration tests | 1d | New test files |

**Total estimated effort**: ~3-4 days of focused implementation.

---

## 6. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| ParallelDialer refactoring introduces dial regression | Medium | High | Comprehensive unit tests for per-scheme routing; preserve existing TCP dial tests as baseline |
| `ResolvedAddress` key change breaks `AddressScore` serialization (if persisted) | Low | Medium | `AddressScore` is in-memory only; no persistence. Confirm via grep. |
| Multi-transport listening complicates port binding errors | Medium | Medium | Each transport failure is independent; log and continue. At least one listener must succeed. |
| WebSocket transport may not be production-ready | Medium | Low | WebSocket activation is Batch 3.6, optional. TCP remains the default. |
| Config-driven registration confuses users | Low | Low | Clear documentation; backward-compatible default (TCP-only if absent). |

---

## 7. Testing Strategy

### 7.1 Unit Tests

```rust
// crates/syncthing-net/src/dialer/tests.rs

#[tokio::test]
async fn test_per_scheme_connector_routing() {
    let mut registry = TransportRegistry::new();
    registry.register(Arc::new(FakeTcpTransport));
    registry.register(Arc::new(FakeWsTransport));

    let dialer = ParallelDialer::with_registry(device_id, "test".into(), Arc::new(registry));

    // TCP address → FakeTcpTransport used
    let tcp_addr = ResolvedAddress::new("tcp", "127.0.0.1:22001".parse().unwrap());
    // WS address → FakeWsTransport used
    let ws_addr = ResolvedAddress::new("ws", "127.0.0.1:22002".parse().unwrap());

    // Verify correct connector is selected for each scheme
}

#[tokio::test]
async fn test_unsupported_scheme_fails_cleanly() {
    let dialer = ParallelDialer::with_tcp_connector(device_id, "test".into());
    let quic_addr = ResolvedAddress::new("quic", "127.0.0.1:22003".parse().unwrap());

    let result = dialer.dial(peer_id, vec![quic_addr], &tls_config, &device_id).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no transport registered for scheme: quic"));
}
```

### 7.2 Integration Tests

1. **TCP + WebSocket coexistence**: Start a node with both transports, verify two listeners bind to different ports.
2. **Cross-scheme dialing**: Node A (TCP only) connects to Node B (TCP + WebSocket) over TCP. Verify sync works.
3. **Scheme fallback**: Node A tries `ws://` first (fails), falls back to `tcp://` (succeeds).

### 7.3 E2E Validation

- Existing `e2e_handshake` and `e2e_sync` tests must pass unchanged (TCP-only default).
- New `e2e_transport_plugin` test: WebSocket transport between two local nodes.

---

## 8. Future Work (Post-v0.3.0)

| Feature | Description |
|---------|-------------|
| QUIC transport | UDP-based, firewall-friendly (UDP 443). Replaces TCP in restricted networks. |
| Tailscale transport | Direct integration with Tailscale daemon via tsnet (Go) or userspace networking. |
| Unix socket transport | Local-only sync between containers on the same host. |
| Path quality-aware routing | Measure RTT/loss per path, dynamically prefer the best transport. |
| Transport hot-reload | Enable/disable transports at runtime without daemon restart. |

---

## 9. Appendices

### 9.1 Rejected Alternatives

**Alternative A: Keep `SocketAddr` in dialer, encode scheme in port number range**
- Rejected: Fragile, non-obvious, conflicts with IANA port assignments.

**Alternative B: Global connector switch (`set_connector()`) instead of per-scheme**
- Rejected: Only supports one active transport at a time. Breaks multi-transport listening.

**Alternative C: Transport selection at BEP session layer instead of dial layer**
- Rejected: Transport selection must happen before TLS handshake (different transports may have different TLS terminators or no TLS at all).

### 9.2 Related Documents

- [KNOWN_ISSUES.md §8](../../KNOWN_ISSUES.md) — Firewall blocking analysis
- [KNOWN_ISSUES.md §12](../../KNOWN_ISSUES.md) — Dual-node real-network test (baseline for regression)
- `crates/syncthing-core/src/traits/transport.rs` — Transport trait definition
- `crates/syncthing-net/src/transport/mod.rs` — TransportRegistry
- `crates/syncthing-net/src/dialer/mod.rs` — ParallelDialer (current)
- `docs/plans/NEXT_STEPS_2026-05-15.md` §T-Net-2 — Original transport plugin tracking
