# Unwrap/Expect Audit Report (T-F2)

**Date**: 2026-05-11  
**Scope**: `crates/` + `cmd/` (non-test code only)  
**Tool**: `grep -rn "unwrap()\|expect(" --include="*.rs" crates/ cmd/` with `#[test]` / `mod tests` / `fn test_` filtered  
**Target**: < 50 occurrences in non-test code  
**Baseline**: ~735 occurrences  

---

## Distribution by Crate

| Crate | Count | Risk Level | Notes |
|-------|-------|------------|-------|
| `syncthing-net` | 240 | **High** | Connection/session paths, async I/O, STUN, relay |
| `syncthing-db` | 194 | **Medium** | Metadata, KV, block-cache storage layers |
| `syncthing-fs` | 154 | **Medium** | Scanner, filesystem, ignore rules |
| `syncthing-sync` | 79 | **High** | Puller, index handler, sync engine |
| `bep-protocol` | 23 | **Medium** | Handshake, message encode/decode |
| `syncthing` (cmd) | 17 | **Low** | CLI/TUI entry points, mostly config parsing |
| `syncthing-api` | 16 | **Medium** | REST handlers, config store |
| `syncthing-test-utils` | 8 | **Low** | Test harness (acceptable debt) |
| `syncthing-core` | 3 | **Low** | DeviceId, base utilities |
| `syncthing-cli` | 1 | **Low** | rustls provider install |
| **Total** | **735** | — | — |

---

## Risk Classification

### High Risk (async I/O, hot path, network)
- `syncthing-net/src/session.rs` — session teardown unwraps
- `syncthing-net/src/stun.rs` — 46 occurrences, UDP socket operations
- `syncthing-sync/src/puller.rs` — block request failures
- `syncthing-net/src/manager/` — dialer/registry/handle async paths

### Medium Risk (storage, protocol, API)
- `syncthing-db/src/metadata.rs` — 57 occurrences, JSON serialization
- `syncthing-db/src/kv.rs` — 48 occurrences
- `syncthing-fs/src/scanner.rs` — 44 occurrences, file hashing
- `syncthing-fs/src/ignore.rs` — 42 occurrences, pattern matching
- `syncthing-api/src/config.rs` — async store load/save unwraps

### Low Risk (entry point, infallible logic, test utils)
- `cmd/syncthing/src/main.rs` — `rustls::crypto` install (infallible at startup)
- `syncthing-core/src/device_id.rs` — base32 ASCII invariant (fixed ✅)
- `syncthing-test-utils/src/harness.rs` — test infrastructure

---

## Repair Strategy

| Pattern | Replacement |
|---------|-------------|
| `option.unwrap()` in `Result`-returning fn | `option.ok_or(...)?` |
| `result.unwrap()` in `Result`-returning fn | `result.map_err(...)?` |
| `vec.last().unwrap()` after push | `let-else` guard or index access |
| `from_utf8(...).unwrap()` on known ASCII | `unsafe { from_utf8_unchecked(...) }` + SAFETY comment |
| `parse().unwrap()` on literals | `expect("infallible literal")` or direct construction |
| `mutex.lock().unwrap()` | `mutex.lock().expect("poison")` — keep for now |

---

## Batch Plan

| Batch | Crates | Target Count | Effort |
|-------|--------|--------------|--------|
| **Batch 1** (done) | `syncthing-core`, `syncthing-api` | 21 → 17 | 30 min ✅ |
| **Batch 2** | `syncthing-cli`, `syncthing` (cmd), `bep-protocol` | 41 → ~20 | 1 h |
| **Batch 3** | `syncthing-test-utils`, `syncthing-sync` | 87 → ~40 | 2 h |
| **Batch 4** | `syncthing-fs` | 154 → ~60 | 3 h |
| **Batch 5** | `syncthing-db` | 194 → ~80 | 4 h |
| **Batch 6** | `syncthing-net` | 240 → ~100 | 5 h |
| **Final** | All crates | → < 50 | review + `cargo clippy` |

> **Estimate**: ~15 hours of focused refactoring to reach < 50.  
> **Recommendation**: Schedule Batch 2–3 this week; Batch 4–6 next week.

---

## Progress Log

- **2026-05-11**: Fixed 4 occurrences in `syncthing-core` + `syncthing-api` (commit `5d32af0`).
