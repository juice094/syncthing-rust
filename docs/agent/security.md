---
type: Policy
title: Security Guidelines and Accepted Audit Debt
description: syncthing-rust 的威胁模型、关键上限、接受的审计债务与部署安全建议。
resource: ./security.md
tags: [agent, security, audit, threat-model, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# 安全注意事项

---

## 1. 威胁模型内

- BEP 协议解析（长度字段溢出、超大消息）
- TLS 配置（rustls 0.23 + ed25519-dalek）
- 路径遍历：`..`、绝对路径、符号链接逃逸必须被拒绝
- 资源耗尽：连接数、扫描内存、消息大小上限
- REST API 认证：默认 loopback 绑定，API key 鉴权
- RBAC 只读 API key：允许配置独立的只读 key，仅授权 GET/HEAD/OPTIONS 请求

---

## 2. 关键上限

| 参数 | 当前值 | 位置 |
|:---|:---|:---|
| `MAX_BEP_MESSAGE_SIZE` | 128 MiB | `bep-protocol` |
| `MAX_BEP_HEADER_SIZE` | 64 KiB | `bep-protocol` |
| `max_connections` | 1000 | `daemon_runner.rs` |
| `connection_timeout` | 120s | `daemon_runner.rs` |
| `MASS_DELETION_SAFETY_RATIO` | 0.5 | `index_handler.rs` |

---

## 3. 接受的审计债务

以下警告已通过 `.cargo/audit.toml` 和 `deny.toml` 显式接受：

| ID | Crate | 路径 | 原因 |
|:---|:---|:---|:---|
| RUSTSEC-2024-0384 | `instant` | `sled → parking_lot → instant` | native 上是 `std::time::Instant` 薄包装 |
| RUSTSEC-2025-0057 | `fxhash` | `sled` 内部 hash table | 无外部输入直接到达 |
| RUSTSEC-2024-0436 | `paste` | `netdev → netlink-packet-core → paste` | 编译期过程宏，运行时暴露为零 |

**禁止**为消除这些警告而引入 breaking change 的依赖升级。

### 3.1 cargo-deny 状态

- `cargo deny check all`（使用 `deny.toml` 或 `cargo-deny.toml`）**已通过**。
- 历史问题：`cmd/syncthing-tray/Cargo.toml` 缺少 `license` 字段且未声明 `publish = false`，导致 cargo-deny 将其视为需许可证的发布 crate。
- 修复方式：已为 `syncthing-tray` 添加 `license = "MIT"` 与 `publish = false`。

---

## 4. 部署安全建议

- REST API 默认绑定 `127.0.0.1:8385`，不要暴露到公网。
- 将 `config.json` 视为机密（含 admin API key 与只读 API key）。
- 同步目录限制在专用目录，不要选系统根目录。
- 监控 RSS 增长；`FileSystemDatabase` 当前使用无界内存缓存。
- 结构化 JSON 日志输出到 `logs/` 目录，日志文件可能包含文件路径、设备 ID 等敏感信息，需按机密文件处理。
- Relay Server 默认监听 `0.0.0.0:22067`（relay）与 `0.0.0.0:22070`（status），部署时应通过防火墙限制访问范围。
