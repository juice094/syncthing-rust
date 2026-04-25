# Contributing to syncthing-rust

> **Scope**: This project is a single-maintainer Windows workspace. Contributions are welcome, but review bandwidth is limited. Open an issue before large changes.

---

## Prerequisites

- **Rust** 1.85+ (`rustup update stable`)
- **PowerShell** (Windows) or **Bash** (Linux/macOS)
- **Git** with LFS if working with large test fixtures

---

## Quality Gates (Mandatory)

Every PR must satisfy:

```powershell
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

- **Tests**: 279+ passed, 0 failed. New features require tests.
- **Clippy**: 0 warnings. No exceptions.
- **Doc-tests**: Public APIs must have runnable examples or doc comments.

---

## Workflow

1. **Fork** → **Branch** (`feat/<name>` or `fix/<name>`) → **Commit** → **PR**.
2. **Commit messages**: Use [Conventional Commits](https://www.conventionalcommits.org/).
   - `feat(net): add IPv6 multicast to LocalDiscovery`
   - `fix(sync): resolve block size mismatch in ManagerBlockSource`
   - `docs(readme): update quick start for PowerShell`
3. **PR description**: Include "What", "Why", and "How verified".

---

## Architecture Constraints

Read [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md) before modifying core logic.

- **`syncthing-core` is read-only** for downstream crates. Do not add dependencies or change public APIs without an ADR.
- **Rust core modules are not outsourced to sub-agents**. Human or primary-agent authored only.
- **Adapter layers / bridges** must live in independent processes (`cmd/`) and remain zero-intrusive to core crates.

---

## Code Style

- Run `cargo fmt` before committing.
- Prefer `thiserror` / `anyhow` for error handling; avoid bare `unwrap` in production paths.
- Log levels: `trace` for per-block/per-packet noise, `debug` for state transitions, `info` for lifecycle events, `warn` for recoverable anomalies, `error` for user-visible failures.
- Async boundaries: `tokio` only; no `async-std`.

---

## Testing Guidelines

- **Unit tests**: Co-located in `src/` via `#[cfg(test)]`.
- **Integration tests**: `acceptance-tests/` for multi-crate scenarios.
- **Mocking**: Use `syncthing-test-utils::MemoryPipe` for BEP transport tests; avoid network I/O in unit tests.
- **Windows-specific**: UDP port tests use ephemeral ports to avoid `10048` conflicts.

---

## Communication

- **Bug reports**: Include `cargo test --workspace` output, OS version, and minimal reproduction.
- **Feature requests**: Reference the [7-dimension evaluation framework](docs/design/ARCHITECTURE_DECISIONS.md#技术选型评估框架) (SDK maturity, dev efficiency, distribution cost, tech-stack consistency, maintenance cost, dependency risk, type safety).
- **Security issues**: Do not open public issues. Email the maintainer directly.

---

## Frozen Items

The following are **stage-frozen** (high cost / low value at v0.2.0). Do not implement without prior discussion:

- Consensus algorithms
- Reputation systems
- Custom cryptographic channel rebuilds beyond rustls TLS 1.3

See [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md) AD-001 for rationale.
