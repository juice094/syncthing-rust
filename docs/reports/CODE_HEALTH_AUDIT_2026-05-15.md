# Code Health and Decoupling Audit Report

> Audit Date: 2026-05-15
> Version: syncthing-rust v0.2.6 (main @ 904a2ab)

---

## 1. Dependency Structure

### 1.1 Internal Crate Dependencies

syncthing(binary) -> syncthing-api, syncthing-net, syncthing-sync, syncthing-core
syncthing-api -> syncthing-core
syncthing-net -> bep-protocol -> syncthing-core
syncthing-sync -> bep-protocol -> syncthing-core
syncthing-core -> (none, bottom layer)

### 1.2 Decoupling Assessment

- Circular Dependencies: NONE (verified by cargo tree)
- Layer Clarity: GOOD - core at bottom, api/net/sync parallel, binary orchestrates
- API Isolation: GOOD - syncthing-api only depends on syncthing-core, uses traits for net interaction
- Test Utils Coupling: WARN - syncthing-test-utils depends on both net and sync

### 1.3 Duplicate Dependency Versions

- base64 0.21.7 + 0.22.1 -> recommend unify to 0.22.1
- crossterm 0.28.1 + 0.29.0 -> recommend unify to 0.29.0

---

## 2. Code Health

### 2.1 unwrap/expect/panic/unreachable in Production Code (excl tests/benches)

| File | Count | Risk |
|------|-------|------|
| syncthing-db/src/kv.rs | 48 | HIGH |
| syncthing-fs/src/scanner/mod.rs | 43 | HIGH |
| syncthing-db/src/store.rs | 34 | HIGH |
| syncthing-db/src/lib.rs | 22 | MEDIUM |
| syncthing-net/src/derp/server.rs | 19 | MEDIUM |
| syncthing-sync/src/index.rs | 18 | MEDIUM |
| cmd/syncthing/src/main.rs | 16 | LOW |
| syncthing-net/src/transport/tcp.rs | 15 | MEDIUM |
| syncthing-net/src/derp/transport.rs | 13 | MEDIUM |
| syncthing-net/src/tcp_transport.rs | 12 | MEDIUM |
| **Total** | **~280** | **HIGH** |

Trend: Down ~20% from 2026-05-14 audit (~350).

### 2.2 Unbounded Channels

- syncthing-net/src/derp/pipe.rs: 4
- syncthing-net/src/derp/server.rs: 1
- Total: 5 (down 55% from 11)

### 2.3 TODO/FIXME

- Total: 4 files, 4 comments (very low, good maintenance)

### 2.4 File Size Distribution

- >600 lines: 2 files (session/mod.rs 608, connection/mod.rs 606) - recommend splitting
- 500~600 lines: 11 files - at warning threshold
- 300~500 lines: ~25 files - normal
- <300 lines: ~120 files - good
- Total: ~158 .rs files, ~39,105 lines

---

## 3. CI Health

### 3.1 Current CI Status

| Check | Status |
|-------|--------|
| cargo fmt --check | PASS |
| cargo clippy -D warnings | PASS (ubuntu + windows) |
| cargo test --workspace | PASS (ubuntu + windows) |
| cargo audit | PASS |
| File size check | PASS |
| Bench smoke test | PASS |
| Release compile check | ADDED in this commit |
| Documentation compile check | ADDED in this commit |
| e2e test | ADDED in this commit |

---

## 4. Conclusion

- Decoupling: PASS. No circular deps, clear layers, good trait boundaries.
- Code Health: MEDIUM. ~280 unwraps in production (concentrated in db/fs/net); 2 files >600 lines; channel/TODO management good.
- CI Coverage: IMPROVED. Added release-check, doc-check, e2e-test jobs.

Main technical debt: error handling robustness (unwraps) and file size control.
