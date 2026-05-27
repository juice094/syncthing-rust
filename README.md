# 🔄 syncthing-rust

> **Rust implementation of the Syncthing protocol stack**
>
> Zero-runtime-dependency deployment, wire-compatible with the official Go daemon.

---

<p align="center">
  <img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/tests-319%20passed-brightgreen" alt="Tests">
  <img src="https://img.shields.io/badge/clippy-0%20warnings-brightgreen" alt="Clippy">
  <img src="https://img.shields.io/badge/version-v0.2.8-blue" alt="Version">
  <img src="https://img.shields.io/badge/license-MIT-blue" alt="License">
</p>

<p align="center">
  <a href="https://github.com/juice094/syncthing-rust/releases/tag/v0.2.9-rc2">📦 Latest Release: v0.2.9-rc2</a> — Centralized Constants + Transport Plugin RFC + Dual-Node E2E Infrastructure
</p>

---

## 📋 项目简介

A Rust implementation of the [Syncthing](https://syncthing.net/) protocol stack, designed for **zero-runtime-dependency** deployment and wire-compatible interoperability with the official Go Syncthing daemon.

**当前阶段**：Alpha — 核心协议完成，E2E 同步验证通过，生产硬化进行中。

| 里程碑 | 状态 |
|:---|:---:|
| Connection layer stable (12h+ single-node endurance, 0 deadlocks) | ✅ |
| Protocol layer correct (TLS, BEP Hello, ClusterConfig, Index) | ✅ |
| End-to-end file sync (single-file two-node ~12s on loopback) | ✅ |
| Cross-version interop (Rust v0.2.8 ↔ Go v2.1.0) | ✅ |
| WSL2↔Windows dual-node sync | ✅ |
| Real-network Tailscale E2E (Win ↔ Ubuntu) | ✅ |
| P0~P2 CRUD repair (create/modify/delete/rename/.stignore exclusion) | ✅ |
| Runtime safety hardening (hot-reload, log rotation, bounded channels, zero panic) | ✅ |
| 72h real-network endurance test | ⏳ Pending (v0.3.0 admission) |

> Not yet a drop-in Go Syncthing replacement, and not yet enterprise-ready (no FIPS/SM crypto, no audit logging). Suitable for: BEP protocol research, Rust reference reading, controlled experiments, personal private deployment, and contributing fixes.

---

## 🎯 当前状态

| 维度 | 状态 |
|:---|:---|
| BEP Protocol (TLS + Hello + ClusterConfig + Index + Request/Response) | ✅ Codec + handshake verified |
| **End-to-end file sync (A→B actual transfer)** | ✅ Working (`e2e_sync` test passes) |
| File-sync internal modules (puller / scanner / folder_model) | ✅ 309 unit tests passing |
| Network Discovery (Local + Global + STUN + UPnP + Relay v1) | ✅ Implementation complete; ParallelDialer with RTT scoring |
| REST API (read + write, Go-layout compatible) | ✅ Read + write complete |
| Tests | **~314 unit + 5 e2e passing, 2 ignored** |
| Lint | **0 clippy warnings** (incl. `await_holding_lock` + `manual_let_else`) |
| Security audit | **3 unmaintained** upstream transitive deps (accepted debt, see `.cargo/audit.toml`) |
| Binary size | ~12 MB (release, Windows x64) |

### 已知限制

| 限制 | 影响 | 缓解措施 |
|:---|:---|:---|
| ClusterConfig first-handshake 10s timeout | 首连延迟 ~12s | 自动重连二次循环必成功；T3.1b `reconnect_device` API + 60s health check |
| Campus/enterprise firewall blocking BEP TCP 22001 | 直连失败 | Tailscale/Headscale/WireGuard 虚拟覆盖层（已验证） |
| 无主动 Push 调度 | 仅被动响应拉取 | v0.3.0 规划 |
| 无 Web GUI | 仅 TUI | — |
| 无 QUIC transport | — | v0.3.0 P0 |

---

## 📁 项目结构

```
syncthing-rust/
├── cmd/syncthing/          # CLI entry point + TUI
├── crates/
│   ├── syncthing-core/     # DeviceId, FileInfo, VersionVector — stable, read-only boundary
│   ├── bep-protocol/       # BEP Hello, Request/Response, Index, ClusterConfig
│   ├── syncthing-net/      # TCP+TLS, ConnectionManager, dialer, discovery, relay
│   ├── syncthing-sync/     # SyncService, Scanner, Puller, IndexHandler, watcher
│   ├── syncthing-api/      # REST API server (Axum)
│   └── syncthing-db/       # Metadata & block cache abstractions
├── docs/
│   ├── design/             # Active ADRs and network design
│   ├── plans/              # Roadmaps and improvement plans
│   ├── reports/            # Verification reports, implementation summaries
│   └── archive/            # Historical decisions
└── README.md
```

> **Trust boundary**: `syncthing-core` is read-only for downstream crates. See [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md).

---

## 🚀 快速开始

### 环境要求

- Rust 1.85+
- Windows 10+ / Linux / macOS

### 构建与运行

```powershell
# 1. Build release binary (< 1 min on modern hardware)
cargo build --release -p syncthing

# 2. Run with interactive TUI
cargo run --release -p syncthing -- tui

# 3. Or run headless
cargo run --release -p syncthing -- run
```

First run generates an Ed25519 TLS certificate and stores it in `%LOCALAPPDATA%\syncthing-rust`.

Default ports: BEP `22001`, REST API `8385`. Loopback addresses bypass API key auth for local debugging.

### 验证它工作

```powershell
# Check REST health
curl http://127.0.0.1:8385/rest/system/status | ConvertFrom-Json

# Expected: uptime > 0, folders/devices counts match your config
```

### 压力测试与韧性验证

```powershell
# Quick 5-minute validation run
cargo run --release -p syncthing --bin stress_test -- --duration 5m --report smoke.csv

# Full 72-hour unattended run with auto-resume on reboot
# See scripts\register-stress-task.ps1 for one-click registration
```

Full tuning plan: [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md).

---

## 🏗️ 架构说明

### 能力矩阵

| 能力 | 状态 |
|:---|:---:|
| Establish TLS-encrypted BEP sessions with Go Syncthing peers | ✅ |
| Pull files block-by-block via Request/Response | ✅ |
| Passively serve block requests (upload) | ✅ |
| Scan local folders, compute SHA-256, broadcast IndexUpdate | ✅ |
| Watch filesystem changes (notify + 1s debounce → ~2s broadcast) | ✅ |
| Discover peers (LAN UDP / Global HTTPS mTLS / STUN / UPnP / Relay v1) | ✅ |
| Parallel dialer (direct TCP vs Relay with RTT scoring) | ✅ |
| REST API (Go-layout compatible, read + write) | ✅ |
| TUI real-time sync state via event bridge | ✅ |
| Hot-reload config.json without restart | ✅ |

### 不做（yet）

| 能力 | 规划 |
|:---|:---|
| Active Push scheduling | v0.3.0 |
| Web GUI | — |
| QUIC transport | v0.3.0 P0 |
| Production packaging (systemd / MSI) | v0.3.0+ |

---

## 📊 路线图

| Phase | 目标 | 状态 |
|:---|:---:|:---|
| **Phase 1** | Core BEP protocol (TLS, Hello, ClusterConfig, Index) | ✅ |
| **Phase 2** | Network abstraction, watcher, REST API, dual-node coexistence | ✅ |
| **Phase 3** | BepSession observability, Push/Pull E2E with remote peer | ✅ |
| **Phase 3.5** | Connection stability, config persistence | ✅ |
| **Phase 4** | TUI hardening (event bridge, live sync state, config hot-reload) | ✅ |
| **Phase 5** | Zero-Tailscale interconnection (discovery → ConnectionManager address pool) | ✅ |
| **Phase A** | Security debt acceptance (cargo audit) | ✅ |
| **Phase B** | 72h stress test | 🟡 Single-node 12h validated; dual-node real-network 功能验证通过 |
| **Phase C** | REST API write-path closure | ✅ |
| **Phase D** | Observability infrastructure (Prometheus metrics) | 🔵 `/metrics` endpoint implemented |

Design docs: [`docs/design/NETWORK_DISCOVERY_DESIGN.md`](docs/design/NETWORK_DISCOVERY_DESIGN.md)  
Execution plan: [`docs/plans/POST_V0_2_0_ROADMAP.md`](docs/plans/POST_V0_2_0_ROADMAP.md)  
Plan index: [`docs/plans/INDEX.md`](docs/plans/INDEX.md)

---

## 📚 文档索引

| 文档 | 用途 |
|:---|:---|
| [`docs/README.md`](docs/README.md) | 文档导航 |
| [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md) | Architecture Decision Records (ADRs) |
| [`docs/design/NETWORK_DISCOVERY_DESIGN.md`](docs/design/NETWORK_DISCOVERY_DESIGN.md) | Network discovery layer design |
| [`docs/reports/IMPLEMENTATION_SUMMARY.md`](docs/reports/IMPLEMENTATION_SUMMARY.md) | Crate-level implementation status |
| [`docs/reports/CRUD_REPAIR_E2E_2026-05-22.md`](docs/reports/CRUD_REPAIR_E2E_2026-05-22.md) | P0~P2 CRUD repair and E2E validation |
| [`docs/reports/WSL2_WINDOWS_DUAL_NODE_E2E_2026-05-18.md`](docs/reports/WSL2_WINDOWS_DUAL_NODE_E2E_2026-05-18.md) | WSL2↔Windows loopback dual-node sync |
| [`docs/reports/REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md`](docs/reports/REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md) | Real-network Tailscale dual-node E2E |
| [`docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md`](docs/reports/VERIFICATION_REPORT_BEP_2026-04-11.md) | BEP interoperability test report |
| [`docs/design/FEATURE_COMPARISON.md`](docs/design/FEATURE_COMPARISON.md) | Feature parity with Go Syncthing |
| [`docs/plans/INDEX.md`](docs/plans/INDEX.md) | Plan document navigation |
| [`docs/plans/PLAN_AUDIT_2026-04-27.md`](docs/plans/PLAN_AUDIT_2026-04-27.md) | Plan validity audit and stage recalibration |
| [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md) | Performance / stability / architecture-debt tuning |
| [`docs/ai-protocol.md`](docs/ai-protocol.md) | Cross-session state anchor for AI agents |

> 性能与调优状态的完整细节见 [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md)。

---

## 🔧 开发验证

```powershell
# Quick validation
cargo test --workspace          # must pass: ~319 passed
cargo clippy --workspace --all-targets  # must be 0 warnings

# Or run the local health check script (Windows)
.\scripts\check-health.ps1
```

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for full contribution guidelines.

---

## 📄 License

[MIT License](./LICENSE).

---

<p align="center">
  <sub>Built with Rust · 319 tests · Wire-compatible with Syncthing · Zero runtime deps</sub>
</p>
