# Agent 环境指引 · syncthing-rust

## 项目定位

`syncthing-rust` 是 P2P 文件同步的 Rust 替代实现，已与官方 Go Syncthing 完成 BEP 协议互操作验证。

- **当前状态**：v0.2.0 Beta，255+ tests，0 TODOs，**0 clippy warnings**
- **传输层**：TCP+TLS / HTTP CONNECT 代理 / SOCKS5 代理 / DERP 中继（自研协议）/ UPnP（PCP/NAT-PMP 骨架待实现）
- **发现层**：Local Discovery（UDP 广播骨架）⚠️ / STUN（公网 IP 查询）⚠️ / PortMapper（UPnP 主路径）⚠️ / **Global Discovery ❌** / **官方 Relay Protocol ❌**
- **同步**：Pull 已验证；被动响应块请求（上传）已实现；主动 Push 调度待完善
- **互操作**：与官方 Go Syncthing 的 BEP 核心消息（Hello/ClusterConfig/Index/Request/Response）在 Tailscale 环境下已验证
- **观测**：REST API（兼容 Go 布局）+ 文件系统 watcher(1s debounce) + TUI

## 架构讨论摘要（来自 2026-04-24 会话）

以下为本项目相关的粗粒度架构决策与待探索方向。

### 1. 分布式验证网络（BEP 扩展）

在现有 BEP 协议上预留 **Verify** 消息族，支持跨实例元认知审计：

- `CrossVerifyRequest` / `CrossVerifyResponse`：对审计结论进行交叉验证
- `LimitDiscoveryBroadcast`：广播新发现的边界限制
- `ManagementStrategyVote`：对限制的管理策略投票
- `ConsensusAchieved`：共识达成通知

**决策**：复用现有 `ReliablePipe` 传输；只定义消息类型与握手流程，不写共识算法。

### 2. 跨实例共识机制

- 共识阈值：2/3 多数
- 最大 quorum 大小：5（防止网络拥塞）
- 实例信誉：本地统计历史验证准确率 × 响应及时性
- 未达共识时：降级为 P2 交付 + 标记"分布式验证未决"

### 3. 边界图谱同步

- `BoundaryMap` 的版本快照通过 syncthing-rust P2P 网络同步
- 单实例发现的限制惠及全网
- 与 clarity-wire 事件总线衔接：L4 元认知引擎通过事件总线广播，syncthing-rust 网关转发到 P2P 网络

### 4. 实例发现与信任模型

- 基于现有 Device ID 机制扩展
- `CapabilityManifest`：声明 L4 版本、边界图谱大小、已管理限制比例、专长领域
- `TrustType`：直接信任 / 间接信任（第三方背书）/ 临时信任

## 当前粗粒度待办

1. 输出 BEP 扩展的 `Verify` 消息类型草案
2. 输出跨实例发现与握手流程图
3. **不做**：共识算法实现、信誉系统、加密信道重建

## 技术选型评估框架

本项目所有技术选型（语言、协议、架构模式）遵循以下七维加权评估：

| 维度 | 说明 | 高权重场景 |
|------|------|-----------|
| **SDK/生态成熟度** | 第三方库稳定性、文档完整性、社区活跃度 | 引入新协议/标准时 |
| **开发效率** | 从原型到可运行的时间成本 | 实验性/验证性功能 |
| **分发/运维成本** | 目标平台的部署复杂度、运行时依赖 | 面向终端用户的功能 |
| **技术栈一致性** | 与现有代码库的语言、工具链、CI 对齐度 | 长期维护的核心功能 |
| **维护成本** | 同一套工具链、技能树、CI 流程能否覆盖 | 单人维护的项目 |
| **依赖风险** | 第三方库 breaking change、协议过时风险 | 核心链路功能 |
| **类型安全/正确性** | 编译期保障对业务正确性的贡献 | 复杂业务逻辑 |

### 决策规则

1. **高必要功能**（缺了项目不可用）：必须满足 **技术栈一致性** + **依赖风险** 双高分，允许牺牲开发效率。
2. **低必要高价值功能**（锦上添花）：允许牺牲 **技术栈一致性**，但 **分发成本** 必须低（独立进程/可选安装）。
3. **适配层/Bridge**：永远 **独立进程**，**零侵入核心**，协议过时了只换 Bridge 不动核心。

### 应用案例：MCP Bridge 语言选型

| 维度 | Python | Rust | 胜出 |
|------|--------|------|------|
| SDK 成熟度 | ★★★★★ | ★★☆☆☆ | Python |
| 开发效率 | ★★★★★ | ★★★☆☆ | Python |
| 分发成本 | ★★☆☆☆ | ★★★★★ | **Rust** |
| 技术栈一致性 | ★☆☆☆☆ | ★★★★★ | **Rust** |
| 维护成本 | ★★★☆☆ | ★★★★☆ | **Rust** |
| 依赖风险 | ★★★★☆ | ★★★☆☆ | Python |
| 类型安全 | ★★☆☆☆ | ★★★★★ | **Rust** |

**结论**：Rust。本项目是 Windows 单人维护、零运行时依赖优先、技术栈全 Rust 的生态，分发成本和技术栈一致性权重压倒 SDK 成熟度。

**实现策略**：手写 JSON-RPC 2.0 协议层（~200 行），不依赖第三方 MCP SDK，只使用工作区已有依赖（tokio/serde_json/reqwest），完全可控、零额外依赖风险。

## 阶段性进展（2026-04-25 Session）

### 已完成

| 模块 | 内容 | 状态 |
|------|------|------|
| docs 重组 | 归档历史文档，建立 design/plans/reports/archive 分层结构 | ✅ |
| BEP 互操作修复 | `client_name` → "syncthing"，`WireFolder.label` → `String`，`validate_device_id` Base32-Luhn | ✅ |
| Local Discovery | 模块拆分为 `discovery/{mod,local,events}.rs`，`daemon_runner.rs` 集成 `run()` + auto-dial | ✅ |
| STUN/PortMapper | `daemon_runner.rs` 启动时接入后台公网地址检测与端口映射 | ✅ |
| clippy | 修复全部 workspace warnings（含手动修复 4 个 + auto-fix 6 个） | ✅ 0 warnings |
| UDP 测试 | `test_udp_broadcast_roundtrip` 改用临时端口，消除 Windows 10048 冲突 | ✅ |

### 当前状态

- **Local Discovery**：UDP 广播发送/接收、protobuf 编解码、auto-dial 已集成；缺少 IPv6 多播、网卡枚举、广播地址计算
- **STUN/PortMapper**：STUN 仅能查询公网映射地址，无 NAT 类型检测、无 hole punching；PortMapper 仅 UPnP 路径可用，PCP/NAT-PMP 未实现，daemon 中无自动续约
- **BEP 互通**：`WireFolder.label` 和 `client_name` 兼容性修复已提交；此前仅在 Tailscale 环境下与 Go 节点验证通过，当前无 Tailscale 时跨网络互联能力为零

### 阻塞项

- **格雷端网络**：Go Syncthing 未监听 Tailscale IP (`100.99.240.98:22000`)，Rust 端 dial 被拒绝 (os error 10061)
- **下一步**：格雷确认 Go 节点运行状态及监听地址，或提供可用地址

## 当前粗粒度待办（更新）

1. 格雷端 BEP 互通验证（修复后的首次完整握手 + 文件同步）
2. 实现 **Global Discovery** 客户端（HTTPS mTLS + JSON，P2 优先级）
3. 实现 **Syncthing Relay Protocol** 客户端（XDR 编解码，P1 优先级）
4. 输出 BEP 扩展的 `Verify` 消息类型草案
5. 输出跨实例发现与握手流程图
6. **不做**：共识算法实现、信誉系统、加密信道重建

## 跨项目接口

- **clarity**：clarity-wire 事件总线 → syncthing-rust P2P 网关 → 跨实例验证
- **devbase**：`.syncdone` 标记格式已对齐；边界图谱版本通过 P2P 同步后写入 devbase OpLog
- **syncthing-mcp-bridge**（独立进程）：Kimi/Claude ← MCP stdio ← Bridge ← REST API → syncthing-rust
