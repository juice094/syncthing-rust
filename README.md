<div align="center">

# 🔄 syncthing-rust

> **Syncthing BEP 协议的 Rust 实现**

[English](./README.md) · [中文](./README-zh.md)

零运行时依赖 · 与 Go 版 Syncthing 线路兼容 · 单静态二进制（~13 MB）

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![CI](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml)
[![Tests](https://img.shields.io/badge/tests-385%20passed-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Version](https://img.shields.io/badge/version-v3.0.3-blue)](https://github.com/juice094/syncthing-rust/releases/tag/v3.0.3)
[![License](https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue)](./LICENSE)

</div>

---

## 📋 简介

[Syncthing](https://syncthing.net/) BEP 协议的 Rust 实现 — 零运行时依赖，单静态二进制，与官方 Go 守护进程线路兼容。

**当前阶段**：Production。核心协议、端到端同步、Windows 系统托盘与 TUI 均已稳定运行。

> [最新发布：v3.0.3](https://github.com/juice094/syncthing-rust/releases/tag/v3.0.3)

---

## 📚 目录

- [核心亮点](#-核心亮点)
- [技术栈](#-技术栈)
- [项目结构](#-项目结构)
- [已知限制](#-已知限制)
- [快速开始](#-快速开始)
- [参与贡献](#-参与贡献)
- [社区与支持](#-社区与支持)
- [许可证](#-许可证)

---

## 🌟 核心亮点

| 亮点 | 说明 |
|:---|:---|
| 🔐 **完整 BEP 协议** | prost 编解码, TLS + Hello + ClusterConfig + Index, LZ4 压缩 |
| 📁 **端到端文件同步** | 块级拉取/推送, SHA-256 扫描, 双向同步 ~1 秒检测到变更 |
| 🚀 **主动 Push** | 本地文件变更立即推送到已连接对端 |
| 🗂️ **版本控制** | Simple (keep=N) + Staggered (4 时间窗口), `.stversions/` 归档 |
| 🌐 **多路径发现** | LAN UDP · 全球 HTTPS mTLS · STUN · UPnP · Relay v1 |
| 🖥️ **实时 TUI** | 事件桥接实时同步状态, 配置热重载 |
| 🔌 **Go 互操作** | 与 Go Syncthing v2.1.0 线路兼容（跨版本已验证） |
| ⚙️ **Folder Orchestrator** | 多文件夹扫描/拉取统一调度，并发限制 + 抖动 + 优先级 |
| 🔮 **预测性健康检查** | 实时评估失败率 / watcher 丢事件 / 状态翻转，自动节流 |
| 📈 **自适应拉取并发** | 基于块请求 RTT 动态调整 downloads/blocks 并发 |
| 🧩 **增量扫描** | watcher 脏路径集合触发子树/单文件增量扫描，降低大 folder 开销 |
| 📦 **单静态二进制** | release 构建 ~13 MB，零运行时依赖 |

---

## 🔧 技术栈

| 组件 | 技术 |
|:---|:---|
| 协议 | BEP over TLS (prost + LZ4) |
| 异步运行时 | Tokio |
| TLS | rustls + ed25519-dalek |
| 网络 | Tokio + rustls + ParallelDialer + Relay v1 |
| 发现 | UDP 广播 + HTTPS mTLS + STUN + UPnP |
| 存储 | sled（元数据 + 块缓存抽象） |
| REST API | Axum（Go 版布局兼容） |
| TUI | ratatui + crossterm |
| CLI | clap |

---

## 📁 项目结构

```
syncthing-rust/
├── cmd/syncthing/          # CLI 入口 + TUI 主循环 + 守护进程
├── crates/
│   ├── syncthing-core/     # 核心类型（DeviceId, FileInfo, VersionVector）
│   ├── bep-protocol/       # BEP 编解码（prost）+ 握手
│   ├── syncthing-net/      # TCP+TLS, ConnectionManager, 拨号, 发现, Relay
│   ├── syncthing-sync/     # Scanner, Puller, IndexHandler, 文件监控
│   ├── syncthing-fs/       # 文件系统抽象（ignore, scanner, watcher）
│   ├── syncthing-api/      # REST API（Axum, Go 版布局兼容）
│   ├── syncthing-db/       # 元数据与块缓存
│   └── syncthing-versioner/# 文件版本控制（Simple + Staggered）
├── docs/                   # 设计文档, 计划, 报告
└── scripts/                # 健康检查, cloud-deploy, 压力测试
```

### 已知限制

| 限制 | 影响 | 缓解 |
|:---|:---|:---|
| 高延迟/不稳定网络 | 大文件批量传输可能因防火墙断开 | 自动重连 + keepalive；网络优化见 [KNOWN_ISSUES §14](docs/KNOWN_ISSUES.md) |
| 无 Symlink 同步 | 符号链接静默跳过 | 未来版本规划 |
| 无 Web GUI | TUI + 系统托盘 + REST API 为主界面（见 [AGENTS.md](AGENTS.md) 冻结声明） | — |
| 无 QUIC transport | 当前仅 TCP + Relay v1（见 [AGENTS.md](AGENTS.md) 冻结声明） | 未来评估 |

---

## 🚀 快速开始

```bash
git clone https://github.com/juice094/syncthing-rust.git && cd syncthing-rust
cargo build --release

# 初始化配置
cargo run --release -- init

# 启动守护进程
cargo run --release -- run --config-dir ~/.syncthing

# 或启动 TUI
cargo run --release -- tui --config-dir ~/.syncthing
```

首次运行自动生成 Ed25519 TLS 证书。默认端口：BEP `22001`, REST API `8385`。

---

## 🤝 参与贡献

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

提交前请运行：

```bash
cargo test --workspace        # 364 passed / 4 ignored / 0 failed
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
```

---

## 💬 社区与支持

| 场景 | 入口 |
|:---|:---|
| Bug 报告 | [GitHub Issues](https://github.com/juice094/syncthing-rust/issues/new?template=bug_report.md) |
| 功能建议 | [GitHub Issues](https://github.com/juice094/syncthing-rust/issues/new?template=feature_request.md) |
| 使用问题 / 讨论 | [GitHub Discussions](https://github.com/juice094/syncthing-rust/discussions) |
| 安全漏洞 | 见 [SECURITY.md](SECURITY.md)（请勿公开 issue） |
| 商业支持 | 见 [SUPPORT.md](SUPPORT.md) / [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md) |

---

## 📄 许可证

[MIT + Commercial](./LICENSE) · [商业许可](./LICENSE-COMMERCIAL.md)

---

<div align="center">

**[⭐ Star](https://github.com/juice094/syncthing-rust) · [🐛 Issues](https://github.com/juice094/syncthing-rust/issues) · [💬 Discussions](https://github.com/juice094/syncthing-rust/discussions) · [🤝 Contribute](CONTRIBUTING.md)**

</div>
