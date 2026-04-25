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

**不做**: 共识算法实现、信誉系统、加密信道重建。

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
