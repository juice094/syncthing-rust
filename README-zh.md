# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-382%20passed-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Version](https://img.shields.io/badge/version-v0.2.10--rc3-blue)](https://github.com/juice094/syncthing-rust/releases)
[![License](https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue)](./LICENSE)

[Syncthing](https://syncthing.net/) 协议栈的 Rust 实现，设计目标为**零运行时依赖**部署，与官方 Go Syncthing 守护进程线级兼容互操作。

> ⚠️ **当前阶段（2026-06-03 v0.2.10-rc3）**：**谨慎生产 — Phase 0 完成，382 tests / 0 failures，E2E 双向同步已实测**。
>
> - ✅ **连接层稳定**：retry_count 累加、TCP keepalive
> - ✅ **协议层健全**：prost 编解码 + LZ4 压缩、wire_compat 10 tests
> - ✅ **同步链路完整**：Pull/Push 双向、~1 秒检测到推送
> - ✅ **版本控制**：Simple + Staggered、`.stversions/` 归档
> - ✅ **5/5 E2E CRUD 测试通过**
> - ⏳ **72h 耐久测试**仍未完成
>
> **尚未达到 Go Syncthing 的替代水准**，但已适用于：研究 BEP 协议、参考 Rust 实现、辅助同步节点、参与开发。

---

## 概览

| 维度 | 状态 |
|-----------|-------|
| BEP 协议 (TLS + Hello + ClusterConfig + Index + Request/Response) | ✅ prost + LZ4 |
| **端到端文件同步** (双向) | ✅ Push/Pull 双向验证 |
| 文件同步核心模块 (puller / scanner / folder_model) | ✅ 382 tests 全通过 |
| 版本控制 | ✅ Simple + Staggered |
| 连接稳定性 | ✅ retry 累加 + TCP keepalive |
| **Rust↔Rust 双向互通** | ✅ v0.2.10-rc3 ↔ v0.2.10-rc3 E2E 574 文件 |
| **Go Syncthing 互操作** | ✅ v0.2.6 ↔ Go v2.1.0 已验证 |
| **72h 耐久测试** | ⏳ 未完成 |
| **Symlink 同步** | 🔲 未实现 |
| **Web GUI** | ❌ 无（仅 TUI + CLI + REST API） |

---

## 快速开始

```bash
git clone https://github.com/juice094/syncthing-rust.git && cd syncthing-rust
cargo build --release

# 初始化配置
cargo run --release -- init

# 启动守护进程
cargo run --release -- run --config-dir ~/.syncthing

# TUI 模式
cargo run --release -- tui
```

---

## 项目结构

```
crates/
├── syncthing-core/       # 核心类型 (只读给下游)
├── bep-protocol/         # BEP 编解码 (prost) + 握手
├── syncthing-net/        # TCP+TLS, ConnectionManager, 发现, Relay
├── syncthing-sync/       # Scanner, Puller, IndexHandler, watcher
├── syncthing-fs/         # 文件系统抽象 (ignore, scanner)
├── syncthing-api/        # REST API (Axum)
├── syncthing-db/         # 元数据存储
└── syncthing-versioner/  # 版本控制 (Simple + Staggered)
```

---

## 贡献

见 [CONTRIBUTING.md](./CONTRIBUTING.md)。提交前：

```bash
cargo test --all
cargo check --all
```

## 许可证

[MIT + Commercial](./LICENSE) · [商业许可](./LICENSE-COMMERCIAL.md)
