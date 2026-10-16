# Contributing to syncthing-rust

> syncthing-rust is a single-maintainer project (Bus Factor = 1). Your participation matters. Open an issue before large changes — review bandwidth is limited but every well-scoped contribution is valued.

## Project Health

| Metric | Status |
|:---|:---|
| Version | `v3.0.4` — authoritative source: `[workspace.package] version` in the root `Cargo.toml` |
| Tests | See [`docs/agent/testing.md`](docs/agent/testing.md) (single source of truth; also [`CI`](https://github.com/juice094/syncthing-rust/actions)) |
| Clippy | 0 warnings (`-D warnings`, incl. `clippy::await_holding_lock`) |
| License | MIT (dual: commercial available) |
| Rust | 1.85+ |

---

## 5-Minute Quick Start

### Requirements

- **Rust**: 1.85.0+ (`rustc --version`)
- **OS**: Windows 10/11 (primary), Linux/macOS community support

```powershell
git clone https://github.com/juice094/syncthing-rust.git
cd syncthing-rust
cargo build --release
cargo test --workspace
```

### First Experience

```powershell
# Generate device identity + config
cargo run -- init

# Start daemon
cargo run -- run --config-dir ~/.syncthing

# CLI health check
cargo run -p syncthing-cli -- status
```

---

## I Want To Contribute... (Decision Matrix)

| What | Entry | Key Files | Must Read |
|:---|:---|:---|:---|
| **Report bug** | [New Issue](https://github.com/juice094/syncthing-rust/issues/new) | — | [docs/KNOWN_ISSUES.md](docs/KNOWN_ISSUES.md) |
| **Fix bug** | [open issues](https://github.com/juice094/syncthing-rust/issues) | `crates/` matching module | [docs/agent/constraints.md](docs/agent/constraints.md) |
| **Add feature** | Open Issue first | `crates/syncthing-core/src/traits/` | [docs/design/topology.md](docs/design/topology.md) |
| **Improve docs** | Edit `.md` files directly | `README.md`, `docs/agent/`, `docs/design/` | [docs/README.md](docs/README.md) |
| **Refactor** | Open Issue first | — | [docs/agent/constraints.md](docs/agent/constraints.md) "Crate boundary hygiene" |

---

## Code Checklist

Before submitting a PR:

- [ ] `cargo test --workspace` — all green (baseline numbers: [`docs/agent/testing.md`](docs/agent/testing.md))
- [ ] `cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock` — 0 warnings
- [ ] `cargo fmt --all -- --check` — pass (or run `cargo fmt --all`)
- [ ] `cargo doc --no-deps --workspace` — pass
- [ ] `cargo deny check all` — pass
- [ ] New public API has doc comment
- [ ] New error paths are logged (not silent)
- [ ] No production `unwrap()` / `expect()` (test code only)
- [ ] Network-layer changes validated with dual `TestNode` instances

### Commit Convention (Conventional Commits)

```
feat:     New feature
fix:      Bug fix
docs:     Documentation
refactor: Refactor (no behavior change)
test:     Tests
chore:    Build/tooling
perf:     Performance
```

Example:

```
fix(sync): Windows rename fallback with exponential backoff

On Windows, fs::rename(tmp, real) fails with ERROR_SHARING_VIOLATION
when the target is opened by editors/AV/desktop search.
- Add rename_with_retry() with 3-layer fallback
- Unit tests cover normal, target-exists, and conflict scenarios
```

---

## Coding Standards

- Run `cargo fmt` before committing
- Log levels: `trace` for block-level, `debug` for state transitions, `info` for lifecycle, `warn` for recoverable, `error` for failures
- Async: `tokio` only; no `async-std`
- Error handling: prefer `thiserror`/`anyhow`; no bare `unwrap` in production paths
- File size: 600-line soft cap per file

---

## Crate Map

| Crate | Purpose | Must Not |
|:---|:---|:---|
| `syncthing-core` | Traits + types + constants | No internal crate deps / no concrete impls |
| `bep-protocol` | Wire format (prost) | No I/O |
| `syncthing-net` | Transport, sessions, discovery, NAT traversal | No sync logic |
| `syncthing-sync` | Scanner, puller, folder model, conflict resolution | No wire format |
| `syncthing-fs` | Filesystem abstraction, ignore patterns, watcher | No sync state machine logic |
| `syncthing-db` | Metadata + block storage backend | Expose sled-specific APIs |
| `syncthing-api` | REST API + event bus + config store | Hold concrete `ConnectionManagerHandle` / `LocalDatabase` |
| `syncthing-versioner` | File versioning strategies | FS I/O |
| `syncthing-test-utils` | Test harnesses (`MemoryPipe`, `TestNode`) | Used only in tests / dev tools |
| `obsidian-vault-crypto-adapter` | Obsidian vault crypto adapter (E2E tests only) | Never linked into production binaries; dev-dependency only |

> The `cmd/` binaries (`syncthing`, `syncthing-cli`, `syncthing-bench`, `syncthing-mcp-bridge`, `syncthing-tray`)
> consume these crates; `cmd/syncthing` does **not** depend on `syncthing-db` / `syncthing-fs` directly
> (it reaches them through `syncthing-sync`).

---

## Architecture Reference

| Document | Content |
|:---|:---|
| [`AGENTS.md`](AGENTS.md) | Entry page for AI agents: red lines + navigation (detailed rules live in `docs/agent/`) |
| [`docs/agent/index.md`](docs/agent/index.md) | Agent guidance bundle: constraints, testing, security, operations |
| [`docs/agent/testing.md`](docs/agent/testing.md) | Test baseline, E2E requirements, pre-submit checklist |
| [`docs/design/topology.md`](docs/design/topology.md) | Project topology, crate DAG, runtime architecture |
| [`docs/design/AGENT_INTEGRATION.md`](docs/design/AGENT_INTEGRATION.md) | Agent integration surfaces: Skill / MCP Bridge / DSH plugin evaluation |
| [`docs/KNOWN_ISSUES.md`](docs/KNOWN_ISSUES.md) | Authoritative defect register and verification facts |
| [`docs/plans/`](docs/plans/) | Implementation plans and situation reports |
| [`scripts/cloud-deploy.sh`](scripts/cloud-deploy.sh) | Automated cloud deployment |

---

## Architecture Constraints

Read [`docs/agent/constraints.md`](docs/agent/constraints.md) before modifying core logic. A short summary:

- **`syncthing-core` is read-only** for downstream crates. Do not add dependencies or change public APIs without an ADR.
- **Crate boundary hygiene**: Core = traits + types only. No concrete implementations leak into core.
- **Platform-agnostic core**: Symlink, xattr, ownership behind trait abstractions with `#[cfg]` implementations.

---

## Frozen Items

The following are **stage-frozen** per [`docs/agent/constraints.md`](docs/agent/constraints.md). Do not implement without an ADR and prior discussion:

- Consensus algorithms / distributed verification extensions
- Reputation systems
- Custom cryptography beyond rustls TLS 1.3
- QUIC / MagicSocket transport
- Web GUI (permanent freeze: TUI + tray + REST API only)

---

## Communication

- **Bug reports**: [GitHub Issues](https://github.com/juice094/syncthing-rust/issues/new)
- **Feature requests**: [GitHub Discussions](https://github.com/juice094/syncthing-rust/discussions)
- **Commercial support**: See [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md)

Thank you for contributing!
