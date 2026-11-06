<div align="center">
  <img src="./docs/assets/logo-512.png" alt="syncthing-rust logo" width="140" style="border-radius: 20px;">

# syncthing-rust

**Syncthing BEP 协议的 Rust 实现**

零运行时依赖 · 与 Go 版 Syncthing 线路兼容 · 单静态二进制

[English](./README.md) · [中文](./README-zh.md)

<a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust"></a>
<a href="https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml"><img src="https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
<a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue" alt="License"></a>
</div>

---

## 简介

Syncthing BEP 协议的 Rust 实现。提供端到端的文件同步，具备零运行时依赖特性，编译为单静态二进制，并与官方 Go Syncthing
守护进程线路兼容互操作。

当前项目处于 **Production** 阶段。核心协议、端到端同步、Windows 系统托盘及 TUI
均已稳定。关于最新的发版说明与更新日志，请参阅 [Releases](https://github.com/juice094/syncthing-rust/releases) 页面。

---

## 快速开始

**前置要求：** Rust 1.85 或更高版本。

```bash
# 从源码构建
git clone https://github.com/juice094/syncthing-rust.git
cd syncthing-rust
cargo build --release

# 初始化配置
cargo run --release -- init

# 启动守护进程
cargo run --release -- run --config-dir ~/.syncthing
```

首次运行会自动生成 Ed25519 TLS 证书。默认情况下，BEP 协议监听 `22001` 端口，REST API 监听 `8385` 端口。

如需探索 TUI、系统托盘或 Relay Server 等其他命令，请运行：

```bash
cargo run --release -- --help
```

---

## 核心特性

- **完整 BEP 协议**：prost 编解码、TLS 握手、ClusterConfig 以及带 LZ4 压缩的 Index。
- **端到端同步**：块级 Pull/Push、SHA-256 扫描，支持亚秒级变更检测的双向同步。
- **多路径发现**：局域网 UDP、全局 HTTPS mTLS、STUN、UPnP 及 Relay v1。
- **版本控制**：支持 Simple (keep=N) 与 Staggered (时间窗) 策略，具备 `.stversions/` 归档。
- **Go 互操作**：与官方 Go Syncthing 守护进程严格线路兼容。
- **可观测性与集成**：实时 TUI、Prometheus 指标、结构化 JSON 日志以及面向 AI Agent 的 MCP Bridge。

> 关于高级调度（并发限制、预测性健康检查）与部署方案，请参阅技术文档。

---

## 已知限制

- **高延迟/不稳定网络**：大文件批量传输可能受防火墙影响而掉线，已通过自动重连和 Keepalive 机制缓解。
- **不支持软链接同步**：目前会静默跳过软链接。
- **无 Web GUI**：交互界面仅限于 TUI、系统托盘和 REST API。
- **不支持 QUIC 传输**：目前仅支持 TCP 及 Relay v1 传输层。

---

## 文档

为保持 README 的专注与稳定，详细技术文档已分离：

- [项目拓扑与架构](docs/design/topology.md)
- [Agent 开发指引与约束](docs/agent/index.md)
- [已知问题与缺陷登记](docs/KNOWN_ISSUES.md)
- [路线图与计划](docs/plans/INDEX.md)

---

## 贡献

详细指南请参阅 [CONTRIBUTING.md](CONTRIBUTING.md)。在提交 PR 前，请确保以下检查通过：

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
```

---

## 社区与支持

- **Bug 报告与功能建议**：[GitHub Issues](https://github.com/juice094/syncthing-rust/issues)
- **使用问题与讨论**：[GitHub Discussions](https://github.com/juice094/syncthing-rust/discussions)
- **安全漏洞**：见 [SECURITY.md](SECURITY.md)（请勿公开提交 Issue）

---

## 许可证

基于 [MIT + Commercial](./LICENSE) 许可。商业支持请参阅 [LICENSE-COMMERCIAL.md](./LICENSE-COMMERCIAL.md)。