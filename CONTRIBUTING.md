# Contributing to syncthing-rust

> syncthing-rust is a single-maintainer project (Bus Factor = 1). Your participation matters. Open an issue before large changes — review bandwidth is limited but every well-scoped contribution is valued.

## Project Health

| Metric | Status |
|:---|:---|
| Version | v0.2.10-rc3 |
| Tests | 382 passed / 0 failed |
| Clippy | 0 warnings |
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
cargo test --all
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
| **Report bug** | [New Issue](https://github.com/juice094/syncthing-rust/issues/new) | — | This file "Commit Convention" |
| **Fix bug** | [open issues](https://github.com/juice094/syncthing-rust/issues) | `crates/` matching module | [AGENTS.md](AGENTS.md) |
| **Add feature** | Open Issue first | `crates/syncthing-core/src/traits/` | Architecture below |
| **Improve docs** | Edit `.md` files directly | `README.md`, `AGENTS.md` | — |
| **Refactor** | Open Issue first | — | [AGENTS.md](AGENTS.md) "Crate boundary hygiene" |

---

## Code Checklist

Before submitting a PR:

- [ ] `cargo test --all` — all green
- [ ] `cargo check --all` — zero warnings
- [ ] New public API has doc comment
- [ ] New error paths are logged (not silent)
- [ ] No production `unwrap()` (test code only)

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
| `syncthing-core` | Traits + types only | No concrete impls |
| `bep-protocol` | Wire format (prost) | No I/O |
| `syncthing-net` | Transport, sessions, discovery | No sync logic |
| `syncthing-sync` | Scanner, puller, folder model | No wire format |
| `syncthing-fs` | Filesystem abstraction | No sync logic |
| `syncthing-db` | Storage backend | No sync logic |
| `syncthing-api` | REST + WebSocket | No sync logic |
| `syncthing-versioner` | File versioning | No FS I/O |

---

## Architecture Reference

| Document | Content |
|:---|:---|
| [`AGENTS.md`](AGENTS.md) | Project conventions, safety rules, progress log |
| [`docs/plans/`](docs/plans/) | Implementation plans and situation reports |
| [`scripts/cloud-deploy.sh`](scripts/cloud-deploy.sh) | Automated cloud deployment |

---

## Architecture Constraints

Read [`AGENTS.md`](AGENTS.md) before modifying core logic.

- **`syncthing-core` is read-only** for downstream crates. Do not add dependencies or change public APIs without an ADR.
- **Crate boundary hygiene**: Core = traits + types only. No concrete implementations leak into core.
- **Platform-agnostic core**: Symlink, xattr, ownership behind trait abstractions with `#[cfg]` implementations.

---

## Frozen Items

The following are **stage-frozen** (high cost / low value at v0.2.x). Do not implement without prior discussion:

- Consensus algorithms
- Reputation systems
- Custom cryptography beyond rustls TLS 1.3

---

## Communication

- **Bug reports**: [GitHub Issues](https://github.com/juice094/syncthing-rust/issues/new)
- **Feature requests**: [GitHub Discussions](https://github.com/juice094/syncthing-rust/discussions)
- **Commercial support**: See [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md)

Thank you for contributing!
