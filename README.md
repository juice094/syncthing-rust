<div align="center">

# 🔄 syncthing-rust

> **Syncthing 协议栈的 Rust 实现**

零运行时依赖 · 与 Go 版 Syncthing 线路兼容 · 单静态二进制（~13 MB）

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-382%20passed-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Version](https://img.shields.io/badge/version-v0.2.10--rc3-blue)](https://github.com/juice094/syncthing-rust/releases)
[![License](https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue)](./LICENSE)

</div>

---

## 📋 简介

[Syncthing](https://syncthing.net/) BEP 协议的 Rust 实现 — 零运行时依赖，单静态二进制，与官方 Go 守护进程线路兼容。

**当前阶段**：Alpha。核心协议完成，端到端同步验证通过，生产硬化进行中。

> [最新发布：v0.2.10-rc3](https://github.com/juice094/syncthing-rust/releases/tag/v0.2.10-rc3) — Phase 0 完成 + 生产就绪修复 + 版本控制

---

## 🌟 核心亮点

| 亮点 | 说明 |
|:---|:---|
| 🔐 **完整 BEP 协议** | prost 编解码, TLS + Hello + ClusterConfig + Index, LZ4 压缩 |
| 📁 **端到端文件同步** | 块级拉取/推送, SHA-256 扫描, 双向同步 ~1 秒检测到推送 |
| 🔄 **主动 Push** | 本地文件变更立即推送到已连接对端 |
| 🗂️ **版本控制** | Simple (keep=N) + Staggered (4 时间窗口), `.stversions/` 归档 |
| 🌐 **多路径发现** | LAN UDP · 全球 HTTPS mTLS · STUN · UPnP · Relay v1 |
| 🖥️ **实时 TUI** | 事件桥接实时同步状态, 配置热重载 |
| 🔄 **Go 互操作** | 与 Go Syncthing v2.1.0 线路兼容 (跨版本已验证) |

---

## 🔧 技术栈

| 组件 | 技术 |
|:---|:---|
| 协议 | BEP over TLS (prost + LZ4) |
| 网络 | Tokio + rustls + ParallelDialer |
| 发现 | UDP 广播 + HTTPS mTLS + STUN + UPnP + Relay v1 |
| 存储 | 元数据与块缓存抽象 |
| REST API | Axum (Go 版布局兼容) |
| TUI | 自定义事件桥接 |

---

## 📁 项目结构

```
syncthing-rust/
├── cmd/syncthing/          # CLI 入口 + TUI 主循环
├── crates/
│   ├── syncthing-core/     # 核心类型 (DeviceId, FileInfo, VersionVector)
│   │                       # 信任边界：对下游 crate 只读
│   ├── bep-protocol/       # BEP 编解码 (prost) + 握手
│   ├── syncthing-net/      # TCP+TLS, ConnectionManager, 拨号, 发现, Relay
│   ├── syncthing-sync/     # Scanner, Puller, IndexHandler, 文件监控
│   ├── syncthing-fs/       # 文件系统抽象 (ignore, scanner, watcher)
│   ├── syncthing-api/      # REST API (Axum, Go 版布局兼容)
│   ├── syncthing-db/       # 元数据与块缓存
│   └── syncthing-versioner/# 文件版本控制 (Simple + Staggered)
├── docs/                   # 设计文档, 计划, 报告
└── scripts/                # 健康检查, cloud-deploy, 压力测试
```

### 已知限制

| 限制 | 影响 | 缓解 |
|:---|:---|:---|
| 无 Symlink 同步 | 符号链接静默跳过 | v0.3.0 规划 |
| 无 Web GUI | 仅 TUI | — |
| 无 QUIC transport | 仅 TCP | 规划中 |

---

## 🚀 快速开始

```bash
git clone https://github.com/juice094/syncthing-rust.git && cd syncthing-rust
cargo build --release
cargo run --release -- run --config-dir ~/.syncthing
```

首次运行自动生成 Ed25519 TLS 证书。默认端口：BEP `22001`, REST API `8385`。

---

## 🤝 参与贡献

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

```bash
cargo test --all              # 382 个测试
cargo check --all             # 0 警告
```

---

## 📄 许可证

[MIT + Commercial](./LICENSE) · [商业许可](./LICENSE-COMMERCIAL.md)

---

<div align="center">

**[⭐ Star](https://github.com/juice094/syncthing-rust) · [🐛 Issues](https://github.com/juice094/syncthing-rust/issues) · [🤝 Contribute](CONTRIBUTING.md)**

</div>
