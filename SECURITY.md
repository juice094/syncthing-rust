# Security Policy

> **Scope**: This document describes the security posture, accepted-debt registry, and reporting process for `syncthing-rust`. For unrelated bug reports, use GitHub issues. For sensitive vulnerabilities, see §Reporting below.

---

## Supported Versions

| Version | Supported | Notes |
|---------|-----------|-------|
| `main`  | ✅ | Active development. Receives all fixes. |
| v3.0.4 | ✅ | Current production release. Security hardened. |
| v3.0.3 | ✅ | Receives security fixes. |
| < v3.0.0 | ❌ | Pre-release / alpha; no support. |

---

## Threat Model (in scope)

`syncthing-rust` is a peer-to-peer file synchronization daemon. The following are considered in-scope for security review:

1. **BEP protocol parsing** — malformed/oversized messages, TLV confusion, integer overflow in length fields.
2. **TLS configuration** — certificate validation, cipher selection, downgrade attacks. (Currently rustls 0.23 + ed25519-dalek for device identity.)
3. **Path traversal** in scanner, puller, watcher — ✅ v3.0.4: `validate_remote_name()` defends against `..`, `\0`, absolute paths, empty segments (see `crates/syncthing-sync/src/block_server.rs`).
4. **Resource exhaustion** — `MAX_BEP_MESSAGE_SIZE`, connection counts (✅ v3.0.4: `max_connections` enforced), scanner memory budget.
5. **REST API auth** — loopback bypass behavior, API key handling, CSRF on writes.

**v3.0.4 additions:**
6. **SSRF prevention** — ✅ v3.0.4: `validate_outbound_addr()` filters multicast/unspecified/link-local addresses; relay session addresses additionally filter loopback.
7. **Information leakage** — ✅ v3.0.4: all `eprintln!` content dumps removed from production puller code.
8. **Key file permissions** — ✅ v3.0.4: Unix `set_permissions(0o600)` on `cert.pem`, `key.pem`, `config.json`.

Out of scope:

- Quantum-resistant cryptography (TLS 1.3 + ed25519 is current state-of-the-art for the target scenario).
- Side-channel attacks on the host hardware.
- Compromised local OS (rust binary runs with user privileges and inherits trust boundary).
- SM2/SM3/SM4 (国密) cipher support — blocked by BEP protocol mandate of SHA-256.

---

## Cryptographic Inventory

| Primitive | Implementation | Purpose |
|-----------|----------------|---------|
| TLS 1.3 | `rustls 0.23` | Transport encryption (BEP-over-TLS, WSS-over-TLS) |
| ed25519 | `ed25519-dalek 2.1` | Device identity certificate signing |
| SHA-256 | `sha2 0.10` | File block content addressing (BEP-protocol-mandated) |
| Random | `ring 0.17` / `rand 0.8` | Key generation, API key, session key, nonce |

> **No protocol-layer SHA-256 substitution is permitted** (BEP wire-format compatibility). Local-only block-cache addressing may switch to BLAKE3 in the future.

---

## Accepted Debt Registry

The following `cargo audit` warnings are **accepted as known debt** and tracked here for transparency. They are not actionable from this project without breaking-change upstream upgrades.

| Crate | Version | Warning | Reason for Acceptance | Source |
|-------|---------|---------|------------------------|--------|
| `paste` | 1.0.15 | unmaintained | Compile-time macro only; zero runtime exposure. Upstream chain: `netdev → netlink-packet-core → paste`, all at latest stable. | `.cargo/audit.toml`, POST_V0_2_0 §Phase A |
| `instant` | 0.1.x | unmaintained | Internal to `sled 0.34.7` (storage core). Replacing sled is a >2-day refactor; ADR-002 decided against. | ADR-002 |
| `fxhash` | 0.2.x | unmaintained | Same upstream chain as `instant`. Inherits ADR-002 decision. | ADR-002 |

The registry is enforced by `.cargo/audit.toml` (ignore list). CI runs `cargo audit --no-fetch` and must emit `0 warnings` (ignored entries are not warnings).

Re-evaluation triggers (any one promotes a warning back to actionable):

- A CVE is assigned to the unmaintained crate.
- A semver-compatible replacement appears upstream (e.g., `sled 1.0` stable).
- The project's threat model expands (e.g., multi-tenant deployment).

---

## Security-Relevant Tuning Knobs

These parameters affect attack surface; review when changing:

| Knob | Current | Tuned target | Reference |
|------|---------|--------------|-----------|
| `MAX_BEP_MESSAGE_SIZE` | 128 MiB | 64 MiB (planned) | TUNING_PLAN T-D4 |
| `MAX_BEP_HEADER_SIZE` | 64 KiB | OK | `connection.rs:32` |
| `connection_timeout` | 120 s | direct 60 s / relay 180 s (planned) | TUNING_PLAN T-D4 |
| `heartbeat_interval` | 30 s | OK (chosen for relay NAT, commit `2950f1f`) | `daemon_runner.rs:117` |
| `max_connections` | 1000 | OK for single-node | `daemon_runner.rs:119` |
| Loopback API auth bypass | enabled by default | Acceptable for single-user desktop; expose only with reverse proxy + TLS | `AGENTS.md` §8.4 |

---

## Reporting a Vulnerability

**Do NOT open public GitHub issues for security-sensitive matters.**

- **Preferred**: email the maintainer at the address listed in `Cargo.toml` / `git log --format='%ae' | head -1`.
- **Response window**: best-effort within 7 days. Single-maintainer project — patience appreciated.
- **Disclosure**: coordinated disclosure preferred. Suggest a 30-day embargo unless actively exploited.

Include in your report:

1. Affected version / commit SHA.
2. Reproduction (minimal Rust harness or Wireshark capture for protocol bugs).
3. Suggested fix or workaround if known.

---

## Hardening Checklist (Deployers)

If you run `syncthing-rust` in untrusted environments:

- [ ] Bind REST API to loopback only (`gui.address = "127.0.0.1:8385"`) unless reverse-proxied with TLS.
- [ ] Rotate API key periodically; treat `config.json` as a secret.
- [ ] Restrict folder paths to dedicated directories — no system roots.
- [ ] Monitor `RSS` growth; current `FileSystemDatabase` uses an in-memory cache (see TUNING_PLAN T-C3 for tightening plan).
- [ ] Log rotation is enabled by default (hourly, 7-day retention); verify `log_dir` is on a partition with adequate space.
- [ ] Review `MAX_BEP_MESSAGE_SIZE` for your peer-trust profile; 128 MiB default is generous.

---

## References

- [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md) — ADR-002 sled retention rationale.
- [`docs/plans/POST_V0_2_0_ROADMAP.md`](docs/plans/POST_V0_2_0_ROADMAP.md) — Phase A security-debt acceptance.
- [`docs/plans/TUNING_PLAN_2026-05-11.md`](docs/plans/TUNING_PLAN_2026-05-11.md) — T-D / T-F tightening plan.
- [`.cargo/audit.toml`](.cargo/audit.toml) — enforced ignore list.
