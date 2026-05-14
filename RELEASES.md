# Releases

## v0.2.6 — Runtime Safety Hardening (2026-05-14)

### 🔒 Security & Stability Hotfix

This is a **pure-fix release** with zero new features. It addresses the root causes identified in post-incident review `INC-20260514-001` (config hot-reload watcher entered a tight loop under notify event storms, producing 21 GB of logs in 19 hours).

**Six hardening patches applied (H-1 through H-6):**

| Patch | File(s) | What changed |
|-------|---------|--------------|
| **H-1** | `daemon_runner.rs`, `config.rs` | Config hot-reload now has a **500 ms debounce** (resetting timer) and mtime comparison. Hot-reload logs downgraded from `info!` → `debug!`. |
| **H-2** | `main.rs` | Daemon log rotation switched from **daily → hourly / 100 MB size cap**, preventing single-day log explosion. |
| **H-3** | `syncthing-net`, `syncthing-sync`, `daemon_runner` | All 7 `unbounded_channel()` sites replaced with **bounded `channel(1024)`**. Senders use `try_send`; full queues drop and emit `warn!`. |
| **H-4** | `relay_listener`, `tcp_transport`, `bep_adapter` | Cleaned up dropped-receiver patterns that leaked memory. |
| **H-5** | `events.rs`, `block_cache`, `derp/server`, `registry` | All `panic!` / `unreachable!()` on **external input** converted to `error! + Err(...)`. |
| **H-6** | `daemon_runner`, `discovery_tasks`, `relay_listener`, `events` | Pure `interval.tick()` loops now have `select! { _ = shutdown.changed() => break }` for graceful termination. |

### 🔬 Cross-Version Interoperability Verified

Rust v0.2.6 ↔ Go Syncthing v2.1.0 **fully verified** on local loopback (2026-05-14):

- TLS 1.3 handshake (`TLS_AES_128_GCM_SHA256`) ✅
- BEP Hello exchange ✅
- ClusterConfig exchange ✅
- Index broadcast / receive ✅
- Block Request / Response ✅
- File materialization on receiver ✅

Automation: `scripts/cross_version_test.sh` (281 lines, Linux/Windows dual-platform).

### 📊 Quality Metrics

```
cargo test --workspace        → 296 passed, 0 failed, 1 ignored (e2e_sync)
cargo clippy --workspace      → 0 warnings (-D warnings)
cargo machete                 → 0 unused deps (4 removed in this cycle)
criterion benches             → 4 skeletons (device_id, scanner, hash_parallel, encode_decode)
```

### 📦 Binaries

| Platform | Target | File | Size |
|----------|--------|------|------|
| Windows | x86_64-pc-windows-msvc | `syncthing-v0.2.6-x86_64-pc-windows-msvc.exe` | ~12 MB |
| Linux | x86_64-unknown-linux-gnu | `syncthing-v0.2.6-x86_64-unknown-linux-gnu` | ~11 MB |
| Stress test | x86_64-pc-windows-msvc | `stress_test-v0.2.6-x86_64-pc-windows-msvc.exe` | ~6.6 MB |

> **Build from source** (any platform with Rust 1.85+):
> ```bash
> cargo build --release -p syncthing
> ```

### ⚠️ Known Limitations

- **§1 ClusterConfig race**: First handshake may stall ~10 s; auto-reconnect succeeds on second cycle. Mitigated by 60 s session health check. `e2e_sync` integration test marked `#[ignore]` under parallel load.
- **72 h endurance**: Scripts ready (`scripts/72h_*.sh`), Linux deployment pending.
- **Tailscale field validation**: Core discovery integration complete; real-world CGNAT/MTU testing deferred.

### 🔗 References

- Full incident write-up: `docs/plans/NEXT_STEPS_2026-05-14.md` §0
- Known issues registry: `docs/KNOWN_ISSUES.md`
- Tuning plan: `docs/plans/TUNING_PLAN_2026-05-11.md`
- Cross-version test script: `scripts/cross_version_test.sh`
