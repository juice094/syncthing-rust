---
type: ADR Bundle
title: Architecture Decisions
description: 汇总 syncthing-rust 所有粗粒度架构决策（ADR），包括已冻结项、网络韧性设计与未来预留方案。
resource: ./ARCHITECTURE_DECISIONS.md
tags: [design, architecture, adr, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# Architecture Decisions

> **权威状态源**：本文档汇总 syncthing-rust 的所有粗粒度架构决策。
> 详细实现状态见 [`../reports/IMPLEMENTATION_SUMMARY.md`](../reports/IMPLEMENTATION_SUMMARY.md)。
> 网络发现层详细设计见 [`NETWORK_DISCOVERY_DESIGN.md`](NETWORK_DISCOVERY_DESIGN.md)。

---

## AD-001: BEP 协议扩展（Verify 消息族）

**日期**: 2026-04-24  
**状态**: 草案阶段

在现有 BEP 协议上预留 **Verify** 消息族，支持跨实例元认知审计：

- `CrossVerifyRequest` / `CrossVerifyResponse`：对审计结论进行交叉验证
- `LimitDiscoveryBroadcast`：广播新发现的边界限制
- `ManagementStrategyVote`：对限制的管理策略投票
- `ConsensusAchieved`：共识达成通知

**决策**: 复用现有 `ReliablePipe` 传输；只定义消息类型与握手流程，不写共识算法。

**阶段性冻结**: 共识算法实现、信誉系统、加密信道重建。
- **冻结理由**: 当前阶段（v0.2.8 Alpha）核心目标是 P2P 文件同步与官方 Go Syncthing 互操作。上述三项属于 L4 元认知远期扩展，当前投入产出比极低（高实现成本 + 零实际部署场景），待多实例生产验证后再解冻。

---

## AD-002: 跨实例共识机制

**日期**: 2026-04-24  
**状态**: 草案阶段

- 共识阈值：2/3 多数
- 最大 quorum 大小：5（防止网络拥塞）
- 实例信誉：本地统计历史验证准确率 × 响应及时性
- 未达共识时：降级为 P2 交付 + 标记"分布式验证未决"

---

## AD-003: 边界图谱同步

**日期**: 2026-04-24  
**状态**: 草案阶段

- `BoundaryMap` 的版本快照通过 syncthing-rust P2P 网络同步
- 单实例发现的限制惠及全网
- 与 clarity-wire 事件总线衔接：L4 元认知引擎通过事件总线广播，syncthing-rust 网关转发到 P2P 网络

---

## AD-004: 实例发现与信任模型

**日期**: 2026-04-24  
**状态**: 草案阶段

- 基于现有 Device ID 机制扩展
- `CapabilityManifest`：声明 L4 版本、边界图谱大小、已管理限制比例、专长领域
- `TrustType`：直接信任 / 间接信任（第三方背书）/ 临时信任

---

## AD-005: MagicSocket 抽象（未来）

**来源**: improvement-plan.md D1  
**日期**: 2026-04-17  
**状态**: 未开始

设计 `MagicSocket` trait：统一 direct / relay / ICE 路径。

```rust
MagicSocket::dial(device_id) → 自动尝试 direct → ICE → DERP
```

路径质量实时监控和自动切换。

**阻塞**: 需先完成 Global Discovery + 官方 Relay Protocol 客户端。

---

## AD-006: DERP 自动回退（未来）

**来源**: improvement-plan.md D2  
**日期**: 2026-04-17  
**状态**: 未开始

- `ParallelDialer` 在 direct 失败后自动尝试 DERP
- DERP 服务器地址配置（GUI/CLI/config）
- DERP 路径质量评分（比 direct 差，但可用）

**注意**: 当前 `derp/` 模块是自研协议，无法与 Syncthing 官方 Go 节点互通。若需与 Go 互通，必须实现官方 Relay Protocol（XDR）。

---

## AD-007: QUIC 预留（远期）

**来源**: improvement-plan.md D3  
**日期**: 2026-04-17  
**状态**: 未开始

- `QuicTransport` 接口（基于 `quinn`）
- 0-RTT 连接建立
- NAT 穿透友好的 UDP 打洞

---

## AD-008: 移动端混合网络连接韧性（2026-06-16 实战结论）

**来源**: ROG-X / Gray-Cloud / Honor 70 Pro 生产环境排障  
**日期**: 2026-06-16  
**状态**: Phase 1 已实现 / P0

### 问题

在 **Tailscale DERP + Syncthing Relay** 混合网络下，单条长连接 TCP 无法同时满足"稳定保活"与"低延迟传输"：

- Tailscale DERP 会把长空闲 TCP 在约 90 秒内掐断（`i/o timeout`、`software caused connection abort`）。
- Syncthing Relay 为长连接设计，能承受高延迟，但带宽和延迟不如 DERP 直连。
- 当前 `ConnectionManager` 在单条连接上同时跑 BEP 索引和块数据；一旦该连接断开，所有传输必须等重连。

### 观察到的最佳配置（临时 workaround）

- 云端设备地址固定为 `tcp://100.69.11.71:22001`（Tailscale 直连，给云端主动拨手机用）。
- 手机端云端设备地址保持 `dynamic`，让手机通过 **Syncthing Relay** 入站到云端。
- 结果：云端→手机走 Tailscale 直连（快），手机→云端走 Relay（稳），互为补充。

### 已实现（Phase 1）

2. **连接竞争静默期**：`ConnectionManager::begin_handshake` 在 TLS 握手完成后、BEP Hello 前注册握手状态并按 device ID 竞争规则提前结束失败连接；同时 outgoing 拨号在本端 device ID 较小时加入 50~250ms 退让窗口，降低双向 TLS ClientHello 同时发生的概率。
3. **指数退避上限**：`RetryConfig::max_backoff_ms` 已设为 5 分钟。
4. **心跳/keepalive 按路径调优**：`ConnectionManagerConfig.heartbeat_interval` 作为全局可配置基准；relay/proxy/websocket/DERP 路径自动收紧为 `min(config, 30s)`，TCP keepalive 对 relay 路径使用 `30s/5s/2`，直连使用 `60s/10s/3`。
5. **DB 锁死自愈**：新增内部健康看门狗（`cmd/syncthing/src/tui/watchdog.rs`），每 30s 检查 `/rest/system/status`，连续两次失败/超时则 spawn 新进程并退出当前进程，实现自愈重启。

### 仍待未来（Phase 2）

1. **双通道保活**：需要 `ConnectionManager` 支持同设备多连接并按消息类型分流（relay 通道负责心跳/索引，direct 通道负责块数据）。

### 阻塞

- 需要 `ConnectionManager` 支持同设备多连接、按消息类型分流。

---

## 决策记录索引

| AD | 主题 | 状态 | 优先级 |
|----|------|------|--------|
| AD-001 | BEP Verify 消息族 | 草案 | P2 |
| AD-002 | 跨实例共识 | 草案 | P2 |
| AD-003 | 边界图谱同步 | 草案 | P2 |
| AD-004 | 实例发现与信任 | 草案 | P2 |
| AD-005 | MagicSocket | 未开始 | P2（阻塞于网络发现层） |
| AD-006 | DERP 回退 | 未开始 | P2 |
| AD-007 | QUIC 预留 | 未开始 | P3 |
| AD-008 | 移动端混合网络连接韧性 | Phase 1 已实现 | P0 |
