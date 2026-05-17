# syncthing-rust justfile — common development commands

# Default recipe: show help
_default:
    @just --list

# Run all checks (fmt, clippy, test, doc, audit)
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
    cargo test --workspace
    cargo doc --no-deps --workspace
    cargo audit

# Run tests only
test:
    cargo test --workspace

# Run clippy only
clippy:
    cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock

# Run cargo-deny
deny:
    cargo deny check all

# Run tests with all features enabled
test-all-features:
    cargo test --workspace --all-features

# Run bench smoke test (compile + short run)
bench-smoke:
    cargo bench --no-run --workspace
    cargo bench -p syncthing-fs --bench scanner -- \
      --warm-up-time 1 --measurement-time 1 --sample-size 10
    cargo bench -p syncthing-sync --bench puller -- \
      --warm-up-time 1 --measurement-time 1 --sample-size 10
    cargo bench -p bep-protocol --bench encode_decode -- \
      --warm-up-time 1 --measurement-time 1 --sample-size 10

# Run E2E sync test
e2e:
    cargo test --release -p syncthing --test e2e_sync

# Check release build compiles
release-check:
    cargo check --release --workspace

# Format all code
fmt:
    cargo fmt --all

# Generate documentation
doc:
    cargo doc --no-deps --workspace

# Build all release binaries needed for stress testing
build-release:
    cargo build --release --bin syncthing
    cargo build --release --bin syncthing-cli
    cargo build --release --bin syncthing-monitor

# Start two-node real-network stress test (Windows side)
two-node-test remote-peer duration="72h":
    powershell -ExecutionPolicy Bypass -File scripts/two-node-real-network-test.ps1 -RemotePeer {{remote-peer}} -Duration {{duration}}

# Stop two-node test
stop-two-node-test:
    powershell -ExecutionPolicy Bypass -File scripts/stop-two-node-test.ps1

# Clean build artifacts
clean:
    cargo clean
