<div align="center">

# 🔄 syncthing-rust

> **Syncthing BEP 协议的 Rust 实现**

[English](./README.md) · [中文](./README-zh.md)

零运行时依赖 · 与 Go 版 Syncthing 线路兼容 · 单静态二进制（~13 MB）

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![CI](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml)
[![Tests](https://img.shields.io/badge/tests-364%20passed-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Version](https://img.shields.io/badge/version-v3.0.3-blue)](https://github.com/juice094/syncthing-rust/releases/tag/v3.0.3)
[![License](https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue)](./LICENSE)

</div>

---

## 简介

[Syncthing](https://syncthing.net/) 协议栈的 Rust 实现，目标为**零运行时依赖**部署，与官方 Go Syncthing 守护进程线路兼容互操作。

**当前阶段（v3.0.3）**：Production — 385 passed / 4 ignored / 0 failures，E2E 双向同步已实测，Windows 托盘 + TUI 稳定。

- ✅ 连接层：retry 累加、TCP keepalive
- ✅ 协议层：prost 编解码 + LZ4 压缩、wire_compat 10 tests
- ✅ 同步链路：Pull/Push 双向、~1 秒检测到变更
- ✅ 版本控制：Simple + Staggered、`.stversions/` 归档
- ✅ Go 互操作：v3.0.3 ↔ Go Syncthing v2.1.0 已验证

完整路线图见 [`docs/plans/POST_V0_2_0_ROADMAP.md`](docs/plans/POST_V0_2_0_ROADMAP.md)。

---

## 目录

- [概览](#概览)
- [快速开始](#快速开始)
- [项目结构](#项目结构)
- [贡献](#贡献)
- [社区与支持](#社区与支持)
- [许可证](#许可证)

---

## 概览

| 维度 | 状态 |
|:---|:---|
| BEP 协议（TLS + Hello + ClusterConfig + Index + Request/Response） | ✅ prost + LZ4 |
| 端到端文件同步（双向） | ✅ Push/Pull 双向验证 |
| 文件同步核心（puller / scanner / folder_model / orchestrator） | ✅ 385 tests / 0 failed |
| 版本控制 | ✅ Simple + Staggered |
| 连接稳定性 | ✅ retry 累加 + TCP keepalive |
| Rust↔Rust 双向互通 | ✅ v3.0.3 ↔ v3.0.3 E2E 验证 |
| Go Syncthing 互操作 | ✅ v3.0.3 ↔ Go v2.1.0 已验证 |
| Folder Orchestrator（统一调度 / 并发限制 / 抖动 / 优先级） | ✅ 多 folder 扫描/拉取协调 |
| 预测性健康检查（失败率 / 丢事件 / 状态翻转 / 自动节流） | ✅ 事件驱动趋势评估 |
| 自适应拉取并发（RTT 动态调整 downloads/blocks） | ✅ 根据链路质量自动换挡 |
| watcher 增量扫描 | ✅ 脏路径子树/单文件增量扫描 |
| Symlink 同步 | 🔲 未实现（未来规划） |
| Web GUI | ❌ 无（TUI + 托盘 + CLI + REST API，见 AGENTS.md 冻结声明） |

---

## 快速开始

```bash
git clone https://github.com/juice094/syncthing-rust.git && cd syncthing-rust
cargo build --release

# 初始化配置
cargo run --release -- init

# 启动守护进程
cargo run --release -- run --config-dir ~/.syncthing

# 启动 TUI
cargo run --release -- tui --config-dir ~/.syncthing
```

首次运行自动生成 Ed25519 TLS 证书。默认端口：BEP `22001`，REST API `8385`。

---

## 项目结构

```
syncthing-rust/
├── cmd/syncthing/          # CLI 入口 + TUI 主循环 + 守护进程
├── crates/
│   ├── syncthing-core/     # 核心类型（只读给下游）
│   ├── bep-protocol/       # BEP 编解码（prost）+ 握手
│   ├── syncthing-net/      # TCP+TLS, ConnectionManager, 发现, Relay
│   ├── syncthing-sync/     # Scanner, Puller, IndexHandler, watcher
│   ├── syncthing-fs/       # 文件系统抽象（ignore, scanner）
│   ├── syncthing-api/      # REST API（Axum）
│   ├── syncthing-db/       # 元数据存储
│   └── syncthing-versioner/# 版本控制（Simple + Staggered）
├── docs/                   # 设计文档、计划、报告
└── scripts/                # 健康检查、cloud-deploy、压力测试
```

---

## 贡献

详见 [CONTRIBUTING.md](./CONTRIBUTING.md)。提交前：

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
cargo fmt --all -- --check
```

---

## 社区与支持

| 场景 | 入口 |
|:---|:---|
| Bug 报告 | [GitHub Issues](https://github.com/juice094/syncthing-rust/issues/new?template=bug_report.md) |
| 功能建议 | [GitHub Issues](https://github.com/juice094/syncthing-rust/issues/new?template=feature_request.md) |
| 使用问题 / 讨论 | [GitHub Discussions](https://github.com/juice094/syncthing-rust/discussions) |
| 安全漏洞 | 见 [SECURITY.md](SECURITY.md)（请勿公开 issue） |
| 商业支持 | 见 [SUPPORT.md](SUPPORT.md) / [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md) |

---

## 许可证

[MIT + Commercial](./LICENSE) · [商业许可](./LICENSE-COMMERCIAL.md)

---

<div align="center">

**[⭐ Star](https://github.com/juice094/syncthing-rust) · [🐛 Issues](https://github.com/juice094/syncthing-rust/issues) · [💬 Discussions](https://github.com/juice094/syncthing-rust/discussions) · [🤝 Contribute](CONTRIBUTING.md)**

</div>
