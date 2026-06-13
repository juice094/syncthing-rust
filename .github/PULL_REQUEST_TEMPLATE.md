## Summary

<!-- Briefly describe the change and its motivation -->

## Related Issues

<!-- Link to the issue(s) this PR addresses. Use "Fixes #123" to auto-close on merge. -->

## Type of Change

<!-- Mark relevant items with [x] -->

- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing behavior to not work as expected)
- [ ] Documentation update
- [ ] Refactoring / Performance improvement
- [ ] Testing infrastructure

## Scope

<!-- Which crates or subsystems are affected? -->

- [ ] `bep-protocol`
- [ ] `syncthing-net`
- [ ] `syncthing-sync`
- [ ] `syncthing-api`
- [ ] `syncthing-core`
- [ ] `syncthing-fs`
- [ ] `syncthing-db`
- [ ] `syncthing-versioner`
- [ ] `syncthing-test-utils`
- [ ] `cmd/syncthing`
- [ ] `cmd/syncthing-cli`
- [ ] `cmd/syncthing-bench`
- [ ] `cmd/syncthing-mcp-bridge`
- [ ] `cmd/syncthing-tray`
- [ ] Other: ___

## Checklist

<!-- All items must pass before merge. -->

- [ ] `cargo test --workspace` passes locally (364 passed / 4 ignored / 0 failed)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock` reports **0 warnings**
- [ ] `cargo fmt --all -- --check` passes (or I ran `cargo fmt --all`)
- [ ] `cargo doc --no-deps --workspace` passes
- [ ] I have updated relevant documentation (`README.md`, `docs/`, `CHANGELOG.md` if applicable)
- [ ] My changes are covered by existing tests or I have added new tests / E2E coverage
- [ ] If this PR modifies public API or protocol behavior, I have updated the design docs or ADRs

## Testing Notes

<!-- Describe how you tested this change (manual steps, stress test, E2E, etc.) -->

## Screenshots / Logs

<!-- If applicable, add screenshots, TUI captures, or log excerpts to help reviewers understand the change. -->
