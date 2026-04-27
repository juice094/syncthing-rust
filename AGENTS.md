# Agent 环境指引 · syncthing-rust

## 项目定位

`syncthing-rust` 是 P2P 文件同步的 Rust 替代实现，已与官方 Go Syncthing 完成 BEP 协议互操作验证。

- **当前状态**：v0.2.0 Beta，294 tests，3 ignored，**0 clippy warnings**
- **传输层**：TCP+TLS / HTTP CONNECT 代理 / SOCKS5 代理 / DERP 中继（自研协议）/ UPnP（PCP/NAT-PMP 骨架待实现）/ **Relay v1 并行拨号 ✅**
- **发现层**：Local Discovery（UDP 广播骨架）⚠️ / STUN（公网 IP 查询）⚠️ / PortMapper（UPnP 主路径）⚠️ / **Global Discovery（HTTPS mTLS 客户端）✅** / **Relay Protocol v1（XDR + ParallelDialer 集成）✅**
- **同步**：Pull 已验证；被动响应块请求（上传）已实现；主动 Push 调度待完善
- **互操作**：与官方 Go Syncthing 的 BEP 核心消息（Hello/ClusterConfig/Index/Request/Response）在 Tailscale 环境下已验证
- **观测**：REST API 读写端点（兼容 Go 布局）+ 文件系统 watcher(1s debounce) + **TUI 实时状态（event bridge）✅** + **配置热重载 ✅**

## 架构讨论摘要

> **完整架构决策记录**见 [`docs/design/ARCHITECTURE_DECISIONS.md`](docs/design/ARCHITECTURE_DECISIONS.md)。
> 以下仅保留快速参考。

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
3. **阶段性冻结**：共识算法实现、信誉系统、加密信道重建。当前阶段投入产出比过低，待多实例生产验证后解冻。

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

## 阶段性进展（2026-04-26 ~ 2026-04-27 Session）

### 已完成

| 模块 | 内容 | 状态 |
|------|------|------|
| Phase E1: rest.rs 拆分 | 1728 行 → `rest/{mod.rs,folder.rs,device.rs,system.rs,system_ops.rs,db.rs,config.rs}` | ✅ |
| Phase E2: manager.rs 拆分 | 1126 行 → `manager/{mod.rs,config.rs,entry.rs,handle.rs,registry.rs,dialer.rs,events.rs,stats.rs}` | ✅ |
| Phase E3: dead-code 清理 | 消除 `syncthing-api` 未使用字段警告 + `progress.rs` 警告抑制 | ✅ |
| REST API Write | `PUT /rest/config`（merge），`POST /system/{restart,shutdown,pause,resume}`，`POST /db/scan` | ✅ |
| TUI Event Bridge | `tokio::sync::mpsc` 桥接 `SyncEvent` → `TuiEvent`；250ms tick 消费 | ✅ |
| TUI Folder State | Folders tab 实时显示 `Idle/Scanning/Pulling/Error` 状态 + 颜色编码 | ✅ |
| TUI Overview Sync | 底部面板显示全局同步状态摘要 | ✅ |
| Config Hot-reload | `notify` 监听 `config.json` → `sync_service.update_config()` + `TuiEvent::ConfigChanged` | ✅ |
| E2E Test Harness | `TestNode` 临时目录 + 自签证书 + `SyncService` + `ConnectionManager` + REST API | ✅ |
| E2E Handshake Test | `test_two_node_empty_folder_handshake`（TCP+TLS+BEP Hello）通过 | ✅ |
| Phase 3-A Relay Dialer | `ParallelDialer::dial` 统一竞速 direct TCP + Relay URL；RTT 评分共享 | ✅ |
| Phase 5 Discovery→CM | Global Discovery 周期性 query + Local Discovery 地址池更新 → `ConnectionManager::update_addresses` | ✅ |

### 当前状态

- **Local Discovery**：UDP 广播发送/接收、protobuf 编解码、auto-dial 已集成；地址发现后更新 `ConnectionManager` 地址池 ✅；缺少 IPv6 多播、网卡枚举、广播地址计算
- **Global Discovery**：Announce + Query 双通路完整；每 5 分钟 query 配置中的 peers，结果注入 `ConnectionManager` 地址池 ✅
- **STUN/PortMapper**：STUN 仅能查询公网映射地址，无 NAT 类型检测、无 hole punching；PortMapper 仅 UPnP 路径可用，PCP/NAT-PMP 未实现，daemon 中无自动续约
- **BEP 互通**：`WireFolder.label` 和 `client_name` 兼容性修复已提交；此前仅在 Tailscale 环境下与 Go 节点验证通过；Phase 5 完成后无 Tailscale 跨网络互联能力已具备理论条件（需实际网络验证）

### 阻塞项

- **格雷端网络**：Go Syncthing 未监听 Tailscale IP (`100.99.240.98:22000`)，Rust 端 dial 被拒绝 (os error 10061)
- **下一步**：格雷确认 Go 节点运行状态及监听地址，或提供可用地址

## 当前粗粒度待办（2026-04-27 后）

1. 格雷端 BEP 互通验证（修复后的首次完整握手 + 文件同步）——阻塞于格雷端网络状态
2. PortMapper PCP/NAT-PMP 骨架填充（L1-PM2）
3. REST API 子功能补全：`device` pause/resume body 参数、subpath scan、conflict resolve
4. 输出 BEP 扩展的 `Verify` 消息类型草案
5. 输出跨实例发现与握手流程图
6. **阶段性冻结**：共识算法实现、信誉系统、加密信道重建。当前阶段投入产出比过低，待多实例生产验证后解冻。

### 已结项（本轮修复完成）

- ✅ Phase E 架构债务清理：`rest.rs` + `manager.rs` 拆分，dead-code 警告消除
- ✅ `cargo clippy --all-targets`：workspace 0 warnings
- ✅ `cargo test --workspace`：294 passed, 0 failed, 3 ignored

## 跨项目接口

- **clarity**：clarity-wire 事件总线 → syncthing-rust P2P 网关 → 跨实例验证
- **devbase**：`.syncdone` 标记格式已对齐；边界图谱版本通过 P2P 同步后写入 devbase OpLog
- **syncthing-mcp-bridge**（独立进程）：Kimi/Claude ← MCP stdio ← Bridge ← REST API → syncthing-rust

---

## 代码健康与架构约束（2026-04-26 注入）

> 以下规则为硬性约束，任何 PR/Agent 交付物触碰红线 → 必须修正后方可合并。

### 1. 分层耦合红线

| 层级 | 允许依赖 | 禁止依赖 |
|------|----------|----------|
| `syncthing-core` | 无（纯 trait + 类型） | 任何内部 crate |
| `syncthing-api` | `syncthing-core` | `syncthing-net` 具体类型、`syncthing-sync` 具体类型 |
| `cmd/syncthing` | 所有 crates | 无 |

**具体禁令**：
- `ApiState` 禁止直接持有 `ConnectionManagerHandle`、`LocalDatabase` 等具体类型。如需网络/数据库能力，应通过 `syncthing-core::traits` 抽象或新增 trait。
- 新增 API 端点时，若涉及网络/同步操作，必须走 `SyncModel`/`ConfigStore` trait，禁止直接调用 `syncthing-net`/`syncthing-sync` 内部函数。

### 2. 上帝对象与文件规模

- `daemon_runner.rs` 当前 858 行，**禁止继续膨胀**。新增网络组件（如 DERP、WebSocket proxy）时，必须拆分为独立模块（如 `discovery_task.rs`、`relay_task.rs`、`dial_task.rs`）。
- 单文件软上限：**600 行**。超过需拆分时，应在 Plan 阶段明确拆分方案。

### 3. Trait 唯一性

- `syncthing-core::traits::SyncModel` 为 canonical trait。
- `syncthing-sync` 内部禁止再定义同名 `SyncModel` trait。现有双生 trait（`syncthing-sync/src/model.rs`）应在后续重构中合并或重命名。

### 4. 测试策略

- 新增功能必须配套 **集成测试**（`tests/*.rs` 或 `cmd/syncthing/src/bin/stress_test.rs` 场景），禁止仅用 `#[cfg(test)]` 单元测试覆盖端到端行为。
- 网络层改动（如 relay 策略、discovery 逻辑）必须通过 `TestNode` 双实例验证，单实例测试视为无效。

### 5. 依赖与存储抽象

- `syncthing-db` 深度绑定 `sled`，若未来需替换存储后端，新增抽象必须落在 `syncthing-core::traits::BlockStore`，禁止在 `syncthing-db` 内部暴露 sled 特有 API。
- 禁止为消除 `cargo audit` warning 而引入 breaking change 依赖升级；允许接受 unmaintained 警告作为记录债务，但必须在 `docs/plans/` 中留下 ADR。
