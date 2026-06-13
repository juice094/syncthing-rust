# syncthing-rust

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![CI](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/juice094/syncthing-rust/actions/workflows/ci.yml)
[![Tests](https://img.shields.io/badge/tests-364%20passed-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Version](https://img.shields.io/badge/version-v3.0.3-blue)](https://github.com/juice094/syncthing-rust/releases)
[![License](https://img.shields.io/badge/license-MIT%20%2B%20Commercial-blue)](./LICENSE)

[Syncthing](https://syncthing.net/) 协议栈的 Rust 实现，设计目标为**零运行时依赖**部署，与官方 Go Syncthing 守护进程线级兼容互操作。

> ✅ **当前阶段（2026-06-13 v3.0.3）**：**Production — 364 passed / 4 ignored / 0 failures，E2E 双向同步已实测，Windows 托盘 + TUI 稳定**。
>
> - ✅ **连接层稳定**：retry_count 累加、TCP keepalive
> - ✅ **协议层健全**：prost 编解码 + LZ4 压缩、wire_compat 10 tests
> - ✅ **同步链路完整**：Pull/Push 双向、~1 秒检测到推送
> - ✅ **版本控制**：Simple + Staggered、`.stversions/` 归档
> - ✅ **5/5 E2E CRUD 测试通过**
> - ⏳ **72h 耐久测试**：v3.1.0 准入线，基础设施就绪，待实际跑测
>
> 当前适用于：个人/辅助节点 P2P 同步、BEP 协议研究、Rust 实现参考、生产环境谨慎部署。完整替代 Go Syncthing 的路线图见 [`docs/plans/POST_V0_2_0_ROADMAP.md`](docs/plans/POST_V0_2_0_ROADMAP.md)。

---

## 概览

| 维度 | 状态 |
|-----------|-------|
| BEP 协议 (TLS + Hello + ClusterConfig + Index + Request/Response) | ✅ prost + LZ4 |
| **端到端文件同步** (双向) | ✅ Push/Pull 双向验证 |
| 文件同步核心模块 (puller / scanner / folder_model) | ✅ 364 tests / 0 failed |
| 版本控制 | ✅ Simple + Staggered |
| 连接稳定性 | ✅ retry 累加 + TCP keepalive |
| **Rust↔Rust 双向互通** | ✅ v3.0.3 ↔ v3.0.3 E2E 验证 |
| **Go Syncthing 互操作** | ✅ v3.0.3 ↔ Go v2.1.0 已验证 |
| **72h 耐久测试** | ⏳ v3.1.0 准入线 |
| **Symlink 同步** | 🔲 未实现（未来规划） |
| **Web GUI** | ❌ 无（TUI + 托盘 + CLI + REST API，见 AGENTS.md 冻结声明） |

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
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -W clippy::await_holding_lock
```

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
