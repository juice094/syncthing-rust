<div align="center">
  <img src="docs/assets/logo-512.png" width="150" alt="syncthing-rust logo" style="border-radius: 20px;">

# syncthing-rust

**生产级的 Syncthing BEP 协议 Rust 实现。**

零运行时依赖，单静态二进制，与官方 Go 守护进程严格线路兼容。

[English](./README.md) · [文档](docs/design/topology.md) · [参与贡献](CONTRIBUTING.md) · [获取帮助](SUPPORT.md)

<a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust"></a>
<a href="https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/juice094/syncthing-rust/ci.yml?label=CI&color=22c55e" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue" alt="License"></a>
</div>

---

## 项目能力

不同于轻量级的封装，`syncthing-rust` 提供了一套全栈、生产级的 Syncthing 协议实现。

- **线路兼容**：无缝接入现有 Syncthing 生态，已通过官方 Go 守护进程的互操作验证。
- **零依赖部署**：编译为单一静态二进制文件。无运行时，无动态库依赖，开箱即用。
- **企业级同步**：块级 SHA-256 校验、TLS 握手，以及多路径发现（局域网 UDP、全局 mTLS、STUN、UPnP、Relay v1）。
- **可观测性与控制**：实时 TUI、Windows 系统托盘、Prometheus 指标，以及面向 AI Agent 编排的 MCP Bridge。

## 快速开始

需要 Rust 1.85 或更高版本。

```bash
# 从源码构建
git clone https://github.com/juice094/syncthing-rust.git
cd syncthing-rust
cargo build --release

# 初始化配置并运行
cargo run --release -- init
cargo run --release -- run --config-dir ~/.syncthing
```

首次运行会自动生成 Ed25519 TLS 证书。默认情况下，BEP 协议监听 `22001` 端口，REST API 监听 `8385` 端口。

## 技术栈

- **协议**：基于 TLS 的 BEP 协议（prost + LZ4 压缩）。
- **异步运行时**：Tokio。
- **TLS**：rustls + ed25519-dalek。
- **存储**：sled（元数据与块缓存抽象）。
- **REST API**：Axum（兼容 Go 布局）+ RBAC API keys。
- **TUI 与 CLI**：ratatui + crossterm + clap。

## 使用示例

### 轻量验证

最快的验证方式是在同一台机器上启动两个节点，观察文件如何双向同步。

### 生产级场景

在真实部署中，`syncthing-rust` 可作为高韧性的集群节点。它能自动穿透 NAT 发现对端（使用 STUN/Relay），根据 RTT 自适应调整拉取并发度，并暴露
`/metrics` 供 Prometheus 告警；同时通过 TUI 即可实现无 GUI 的实时监控。

## 能力与配置

- **Folder 调度器**：统一的多文件夹扫描/拉取调度，支持并发限制与优先级。
- **版本控制**：支持 Simple (keep=N) 与 Staggered (时间窗) 策略，具备 `.stversions/` 归档。
- **预测性健康检查**：实时评估失败率并自动节流。
- **增量扫描**：通过 Watcher 脏路径集合触发子树或单文件扫描。
- **RBAC API 密钥**：REST API 支持独立的 admin 与只读密钥。

## 已知限制

- **无 Web GUI**：交互界面严格限制为 TUI、系统托盘和 REST API。
- **不支持软链接同步**：目前会静默跳过软链接。
- **不支持 QUIC 传输**：目前仅支持 TCP 及 Relay v1 传输层。
- **高延迟网络**：大文件批量传输可能受防火墙影响而掉线（已通过自动重连和 Keepalive 机制缓解）。

## 文档

为保持 README 的专注，详细技术文档已分离：

- [项目拓扑与架构](docs/design/topology.md)
- [Agent 开发指引与约束](docs/agent/index.md)
- [已知问题与缺陷登记](docs/KNOWN_ISSUES.md)
- [路线图与计划](docs/plans/INDEX.md)

## 开发与测试

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
```

## 参与贡献

欢迎提交 Issue 和 Pull Request。提交前请阅读[贡献指南](CONTRIBUTING.md)。安全问题请参阅[安全策略](SECURITY.md)。

## 许可证

基于 [MIT + Commercial](./LICENSE) 许可。商业支持请参阅 [LICENSE-COMMERCIAL.md](./LICENSE-COMMERCIAL.md)。