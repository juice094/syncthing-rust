<div align="center">

# 🔄 syncthing-rust

> **Syncthing 协议栈的 Rust 实现**

零运行时依赖 · 与 Go 版 Syncthing 线路兼容 · 单静态二进制（~12 MB）

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-319%20passed-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Clippy](https://img.shields.io/badge/clippy-0%20warnings-brightgreen)](https://github.com/juice094/syncthing-rust/actions)
[![Version](https://img.shields.io/badge/version-v0.2.8-blue)](https://github.com/juice094/syncthing-rust/releases)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

</div>

---

## 📋 简介

[Syncthing](https://syncthing.net/) BEP 协议的 Rust 实现 — 零运行时依赖，单静态二进制，与官方 Go 守护进程线路兼容。

**当前阶段**：Alpha。核心协议完成，端到端同步验证通过，生产硬化进行中。尚未成为 Go 版即插即用替代品。

> [最新发布：v0.2.9-rc2](https://github.com/juice094/syncthing-rust/releases/tag/v0.2.9-rc2) — 集中化常量 + 传输插件 RFC + 双节点端到端基础设施

---

## 🌟 核心亮点

| 亮点 | 说明 |
|:---|:---|
| 🔐 **完整 BEP 协议** | TLS + Hello + ClusterConfig + Index + Request/Response，编解码器已验证 |
| 📁 **端到端文件同步** | 块级拉取/推送，SHA-256 扫描，双节点环回 ~12 秒完成传输 |
| 🌐 **多路径发现** | LAN UDP · 全球 HTTPS mTLS · STUN · UPnP · Relay v1，RTT 评分拨号器 |
| 🖥️ **实时 TUI** | 事件桥接实时同步状态，配置热重载无需重启 |
| 🔄 **Go 互操作** | 与 Go Syncthing v2.1.0 线路兼容（跨版本已验证） |

> [完整里程碑与路线图 → docs/plans/INDEX.md](docs/plans/INDEX.md)

---

## 🔧 技术栈

| 组件 | 技术 |
|:---|:---|
| 协议 | BEP over TLS（自定义编解码器） |
| 网络 | Tokio + rustls + 自定义拨号器 |
| 发现 | UDP 广播 + HTTPS mTLS + STUN + UPnP + Relay v1 |
| 存储 | 元数据与块缓存抽象 |
| REST API | Axum（Go 版布局兼容） |
| TUI | 自定义事件桥接 |

---

## 📁 项目结构

```
syncthing-rust/
├── cmd/syncthing/          # CLI 入口 + TUI 主循环
│                           # 首次运行自动生成 Ed25519 TLS 证书
├── crates/
│   ├── syncthing-core/     # 核心类型：DeviceId, FileInfo, VersionVector
│   │                       # 信任边界：对下游 crate 只读
│   ├── bep-protocol/       # BEP 编解码器与握手协议
│   │                       # Hello, ClusterConfig, Index, Request/Response
│   ├── syncthing-net/      # TCP+TLS, ConnectionManager, 拨号器, 发现, Relay
│   │                       # ParallelDialer RTT 评分，多路径并行连接
│   ├── syncthing-sync/     # SyncService, Scanner, Puller, IndexHandler, 文件监控
│   │                       # notify + 1 秒防抖 → ~2 秒广播变更
│   ├── syncthing-api/      # REST API 服务端（Axum，Go 版布局兼容）
│   │                       # 读写路径完整实现
│   └── syncthing-db/       # 元数据与块缓存抽象层
├── docs/
│   ├── design/             # 活跃 ADR 与网络层设计
│   ├── plans/              # 路线图与改进计划
│   ├── reports/            # 验证报告与实现摘要
│   └── archive/            # 历史决策归档
└── scripts/                # 健康检查、压力测试注册脚本
```

### 信任边界

`syncthing-core` 对下游 crate **只读**。这是架构层面的不变量 — 详见 [docs/design/ARCHITECTURE_DECISIONS.md](docs/design/ARCHITECTURE_DECISIONS.md)。

### 已知限制

| 限制 | 影响 | 缓解 |
|:---|:---|:---|
| ClusterConfig 首次握手 10 秒超时 | 首连延迟 ~12 秒 | 自动重连二次循环必成功 |
| 校园/企业防火墙阻断 BEP TCP 22001 | 直连失败 | Tailscale/WireGuard 虚拟覆盖层（已验证） |
| 无主动 Push 调度 | 仅被动响应拉取 | v0.3.0 规划 |
| 无 Web GUI | 仅 TUI | — |

---

## 🚀 快速开始

```bash
# 1. 克隆仓库
git clone https://github.com/juice094/syncthing-rust.git && cd syncthing-rust

# 2. 编译（< 1 分钟）
cargo build --release -p syncthing

# 3. 运行
cargo run --release -p syncthing -- tui    # 交互式 TUI
# 或: cargo run --release -p syncthing -- run  # 无头模式
```

首次运行在 `%LOCALAPPDATA%\syncthing-rust` 生成 Ed25519 TLS 证书。  
默认端口：BEP `22001`，REST API `8385`。

```powershell
# 验证服务正常
curl http://127.0.0.1:8385/rest/system/status | ConvertFrom-Json
```

---

## 🤝 参与贡献

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。快速验证：

```bash
cargo test --workspace        # ~319 个测试
cargo clippy --workspace --all-targets  # 0 警告
```

---

## 📄 许可证

[MIT 许可证](LICENSE)。

---

<div align="center">

**[⭐ Star](https://github.com/juice094/syncthing-rust) · [🐛 Issues](https://github.com/juice094/syncthing-rust/issues) · [🤝 Contribute](CONTRIBUTING.md)**

</div>
