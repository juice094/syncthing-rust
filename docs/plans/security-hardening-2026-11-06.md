---
type: Plan
title: Security & Reliability Hardening Plan
description: 基于代码事实的安全与可靠性缺口改造计划，面向 v3.1.0 Production 准入。
resource: ./security-hardening-2026-11-06.md
tags: [plan, security, reliability, hardening, production]
status: in-progress
project: syncthing-rust
timestamp: 2026-11-06T00:00:00Z
---

# 安全与可靠性加固计划

## 0. 背景与方法

本计划基于 2026-11-06 对 `crates/` 的定向代码检索，不采信 README 或历史文档中的宣称，只以代码事实为准。

方法：先读 `docs/agent/security.md` 建立宣称基线，再用 `search_code` 验证每一条安全机制的真实调用点与实现完整性。

## 1. 诊断基线（代码事实）

| 能力 | 状态 | 代码证据 | 风险 |
|:---|:---|:---|:---|
| 连接数上限 | 已实现 | `manager/mod.rs:360`，`max_connections=1000` | 低 |
| 并发控制（Semaphore） | 已实现 | `puller/mod.rs:201,434`，`orchestrator.rs:177`，`sync_task.rs:197` | 低 |
| 消息大小上限 | 已实现 | `MAX_BEP_MESSAGE_SIZE=128MiB`，`security.md` | 低 |
| 路径穿越防御 | 已实现 | `block_server.rs:66` `validate_remote_name`，调用点 `index_handler.rs:172`、`puller/mod.rs:357,630,673` | 低 |
| SSRF 防御 | 部分 | `tcp_transport.rs:261` `validate_outbound_addr`，拒绝 multicast/unspecified/link-local，但放行 loopback 与私网 | 中 |
| 冲突文件保留 | 已实现 | `conflict_resolver.rs:224`，格式 `name.sync-conflict-<timestamp>-<device short ID>.ext` | 低 |
| 误删保护 | 已实现 | `MASS_DELETION_SAFETY_RATIO=0.5`，`index_handler.rs` | 低 |
| 块级断点续传 | 未发现 | `completed_block|received_block|resume_block|block_state` 零命中 | 高 |
| 字节级带宽限速 | 未实现 | 仅有 `orchestrator.rs` 的 `throttle_percent`（并发节流），无 token bucket / bytes-per-sec | 中 |

## 1.5 实施进度（2026-11-06 更新）

| 项 | 状态 | 实际落地 | 未落地 |
|:---|:---|:---|:---|
| P1-2 SSRF 策略 | 已完成 | `Options` 四字段 + `OutboundPolicy` + `validate_outbound_addr(addr, policy)` + `dialer` 传参 + `daemon_runner` 从 `Config.options` 构建 + 测试重写 8 例 + `security.md` 上限表 | 无 |
| P1-1 带宽限速 | 部分完成 | `throttle.rs` 令牌桶（6 单测）、`Puller.recv_throttle` + `download_file` 写盘前 `acquire`、`FolderModel::set_recv_throttle`、`lifecycle` 按 `Options.max_recv_kbps` 注入、`SyncModel` 两个默认 no-op 方法 | `SyncService` 未覆盖 `set_bandwidth_limits`；`POST /rest/system/throttle` 未做；发送侧（`block_server`）未限速；每设备覆盖未做 |
| P0 块续传 | 未开始 | G1 二次确认成立（`completed_block`/`resume_block`/`block_state`/`pending_block` 零持久化命中） | `BlockProgressStore` trait、`syncthing-db` 实现、`puller` 接入、`GET /rest/db/progress` |

**偏差登记**：ADR-20261106-bandwidth-throttle 的每设备覆盖（`Device.max_recv_kbps`）因 `Device` 有 4 处字面量构造点，本轮不做，留待后续；ADR 状态已改为 `partially-implemented`。ADR-20261106-block-resume-persistence 状态为 `proposed`。ADR-20261106-outbound-address-policy 已实现。

## 2. 缺口分析

### G1：块级续传状态缺失（P0）

`puller` 以 `Semaphore` 控制并发块请求，但接收进度没有落到 `sled`。网络中断或进程重启后，无法只请求缺失块；最坏情况下整文件重新拉取。72h 耐久测试会因长尾重传而不稳定，这是 v3.1.0 准入线的硬阻塞。

**前置确认**：用 `partial|chunk|offset|block_progress` 二次检索，排除命名差异导致的误判。若确认缺失，按 P0 施工。

### G2：SSRF 放行私网与回环（P1）

`validate_outbound_addr` 拒绝 multicast、unspecified、link-local，但放行 `127.0.0.1`、`192.168/16`、`10/8`、`172.16/12`。对局域网发现是合理设计，但恶意 peer 可诱导本机连接内网服务。当前无配置项让用户收紧。

### G3：无字节级带宽限速（P1）

`throttle_percent` 只降低并发度，单连接仍可吃满下行带宽。对多租户或共享链路的部署场景，缺少硬限速能力。

## 3. 改造项

### P0-1：块级续传状态持久化

- **目标**：接收中的文件按块记录已校验块集合，重启后仅请求缺失块。
- **方案**：在 `syncthing-db` 增加 `BlockProgress` 表（`folder_id + file_name + block_index -> hash`），`puller` 写入前查表跳过已校验块，文件完成后清理记录。不暴露 sled 特有 API（遵守 crate 边界红线）。
- **改动面**：`syncthing-db`、`syncthing-sync/src/puller`。不触碰 `bep-protocol` 与 `syncthing-core`。
- **风险**：DB 写放大；需在 `MASS_DELETION_SAFETY_RATIO` 之外新增清理策略，防止孤儿记录堆积。
- **验证**：`TestNode` 双实例，传输中断后重连，断言仅请求缺失块（用请求计数打点）。

### P1-1：SSRF 地址策略可配置

- **目标**：默认保持当前行为（兼容局域网发现），提供 `allow_private_targets: bool`（默认 true）与 `allow_loopback_targets: bool`（默认 false）。
- **方案**：`tcp_transport.rs` 的 `validate_outbound_addr` 接受策略参数，配置从 `Config` 注入。
- **改动面**：`syncthing-net`、`daemon_runner` 配置结构。
- **风险**：默认值变更会破坏现有部署；因此默认值保持现状，仅新增开关。
- **验证**：单测覆盖四种策略组合；集成测试验证拒绝路径返回明确错误码。

### P1-2：字节级带宽限速

- **目标**：每设备可选 `max_recv_kbps` / `max_send_kbps`，0 表示不限。
- **方案**：在 `puller` 的块读取路径与 `block_server` 的块发送路径各挂一个 token bucket（`governor` 或手写 `tokio::time::sleep` 令牌桶），避免引入重型依赖。
- **改动面**：`syncthing-sync`、`syncthing-net`（发送侧）。
- **风险**：限速粒度选错会引入尾延迟；需用 `throttle_percent` 协同，避免双层节流震荡。
- **验证**：单测验证令牌桶速率；集成测试用 `TestNode` 断言单位时间传输量不超过阈值。

## 4. 明确不做（避免范围蔓延）

- 不引入全局分布式锁（违反 AP 定位）。
- 不做信誉系统或 peer 评分（`constraints.md` 冻结项）。
- 不升级 `sled` 大版本（审计债务已显式接受，见 `security.md` 第 3 节）。
- 不改 `MAX_BEP_MESSAGE_SIZE` 默认值（改动会破坏 Go 互操作）。

## 5. 验证策略

- 每项改造必须走 `TestNode` 双实例，单实例测试视为无效（`AGENTS.md` 硬红线 5）。
- P0 需在 72h 耐久测试前完成，否则准入线不成立。
- 提交前五道门全绿：`fmt` / `clippy` / `test` / `doc` / `deny`。

## 6. 文档同步义务

- 改动 `syncthing-net` 策略参数：同步 `docs/agent/security.md` 第 2 节关键上限表。
- 改动 `syncthing-db` 表结构：同步 `docs/design/topology.md` 存储层描述。
- 新增配置项：同步 `docs/agent/claude_reference.md` 的 CLI/REST 全表。
- README 措辞：在 P0/P1 落地前，README 不得出现 "resume"、"rate limiting"、"DoS mitigation"；落地后按实际能力精确表述。

## 7. 验收标准

- P0：断点续传集成测试通过，72h 耐久测试无长尾重传告警。
- P1-1：四种策略组合单测通过，默认行为与当前一致。
- P1-2：限速集成测试通过，`throttle_percent` 与令牌桶无震荡。
- 文档：上述同步义务全部完成，`docs/agent/security.md` 上限表更新。
