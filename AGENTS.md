# Agent 环境指引 · syncthing-rust

> 📋 **当前权威路线图**: [`docs/plans/POST_V0_2_0_ROADMAP.md`](./docs/plans/POST_V0_2_0_ROADMAP.md)
> 📋 **计划索引**: [`docs/plans/INDEX.md`](./docs/plans/INDEX.md)
> 📋 **审计报告**: [`docs/plans/PLAN_AUDIT_2026-04-27.md`](./docs/plans/PLAN_AUDIT_2026-04-27.md)

## 项目定位

`syncthing-rust` 是 P2P 文件同步的 Rust 替代实现。当前验证目标为 **Rust 新版 ↔ Rust 旧版（格雷侧）** 的 BEP 互通；Go Syncthing 互操作待后续验证。

- **当前状态**：v0.2.0 Beta，294 passed / 3 ignored / 0 failed，**0 clippy warnings**，**cargo audit: 3 unmaintained**
- **传输层**：TCP+TLS / HTTP CONNECT 代理 / SOCKS5 代理 / DERP 中继（自研协议）/ UPnP（PCP/NAT-PMP 骨架待实现）/ **Relay v1 并行拨号 ✅**
- **发现层**：Local Discovery（UDP 广播骨架）⚠️ / STUN（公网 IP 查询）⚠️ / PortMapper（UPnP 主路径）⚠️ / **Global Discovery（HTTPS mTLS 客户端）✅** / **Relay Protocol v1（XDR + ParallelDialer 集成）✅**
- **同步**：Pull 已验证；被动响应块请求（上传）已实现；主动 Push 调度待完善
- **互操作**：旧版 Rust syncthing-rust ↔ 新版 Rust 待验证（格雷侧为 pre-fix 构建）；Go Syncthing 互操作待后续验证
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

## 阶段性进展（截至 2026-04-27）

### 已完成（与计划对齐）

| 模块 | 内容 | 来源计划 | 状态 |
|------|------|----------|------|
| Phase 1~2 网络修复 | TCP+TLS+BEP Hello+帧解析；Daemon 启动；Puller 真实块请求 | MVP_RECOVERY | ✅ |
| Phase 3 BepSession 硬化 | Observability/Events/Metrics；Peer Sync State；Push/Pull E2E | PHASE3_PLAN 3.1~3.3 | ✅ |
| Phase 4 兼容性收尾 | 连接循环竞争解决；`.stignore`；配置持久化；身份层解耦 | PHASE4_PLAN 4.2 | ✅ |
| Wave 3 网络基础设施 | NetMonitor 网络变更；ParallelDialer 竞速拨号；Supervisor 监督树 | WAVE3_PLAN | ✅ |
| REST API 读写端点 | `PUT /rest/config`，`POST /system/{restart,shutdown,pause,resume}`，`POST /db/scan` | improvement-plan C1 | ✅ |
| TUI Event Bridge + 热重载 | `SyncEvent` → `TuiEvent`；`notify` 监听 config.json | PHASE4_PLAN 4.1 | ✅ |
| E2E Handshake Test | `test_two_node_empty_folder_handshake`（TCP+TLS+BEP Hello） | MVP_RECOVERY | ✅ |
| Phase E 架构债务 | `rest.rs` 1728→7 模块；`manager.rs` 1126→8 模块；dead-code 清理 | *自发* | ✅ |

### 未完成（计划内阻塞项）

| 模块 | 内容 | 来源计划 | 状态 | 优先级 |
|------|------|----------|------|--------|
| cargo audit 清理 | fxhash/instant/paste 3 项 unmaintained | POST_V0_2_0 Phase A | ⏳ | **P0** |
| 72h Stress Test | 长期运行稳定性验证（内存/连接/同步） | PHASE3_PLAN 3.4 / PHASE4_PLAN 4.3 | ⏳ | **P1** |
| REST API sub-gaps | `device` pause/resume body、subpath scan、override/revert stub | POST_V0_2_0 Phase C | ⏳ | P2 |
| Delta Index 验证 | `IndexID` + `Sequence` 长时间一致性 | PHASE4_PLAN 4.2 | ⏳ | P3 |

### 当前状态

- **Local Discovery**：UDP 广播发送/接收、protobuf 编解码、auto-dial 已集成；地址发现后更新 `ConnectionManager` 地址池 ✅；缺少 IPv6 多播、网卡枚举、广播地址计算
- **Global Discovery**：Announce + Query 双通路完整；每 5 分钟 query 配置中的 peers，结果注入 `ConnectionManager` 地址池 ✅
- **STUN/PortMapper**：STUN 仅能查询公网映射地址，无 NAT 类型检测、无 hole punching；PortMapper 仅 UPnP 路径可用，PCP/NAT-PMP 未实现，daemon 中无自动续约
- **BEP 互通**：`WireFolder.label` 和 `client_name` 兼容性修复已提交；与 Go Syncthing 的验证尚未完成；当前验证目标为新版 Rust ↔ 格雷侧旧版 Rust

### 阻塞项

- **格雷端 BEP 互通验证**：格雷侧运行 **pre-fix Rust 构建**，但**可切换为 Go/Rust 双版本**。新版 dial 旧版被拒绝 (os error 10061)。根因待格雷侧配合排查。
- **格雷侧操作指南**: [`docs/plans/GRAY_SIDE_OPS.md`](./docs/plans/GRAY_SIDE_OPS.md) — 含三种验证方案（同版本Rust/Go/旧版兼容性）、PowerShell自查命令、日志收集要求、决策树。
- **策略**：格雷验证与开发主路径**并行推进**，不阻塞。解阻塞时冻结新功能，全力修 bug。

### 本轮开发窗口（按修正后路线图执行）

**P0: 72h Stress Test 执行**
- 已有 `cmd/syncthing/src/bin/stress_test.rs`（290 行）
- 先本地短周期预验证（2~4h），无异常后移交格雷远程执行 72h

**P0: 跨版本 Rust 互通验证**
- 新版 `main` ↔ 格雷侧 pre-fix Rust
- 解阻塞后冻结新功能，全力修兼容性问题

**P1: REST API 写端闭环**
- `POST /rest/db/override` / `revert` — 从 501 stub 实现
- `POST /rest/system/pause` / `resume` — `device` body 参数生效
- `POST /rest/db/scan` — `sub` 子路径参数支持

**P2: .stignore 简化版审计**
- 评估 `syncthing-sync/src/ignore.rs`（241 行）是否覆盖 90% 场景

**P3: cargo audit 债务接受**
- 创建 `.cargo/audit.toml`，显式接受 3 个 unmaintained 警告
- 不再视为主动开发任务

### 本轮开发窗口（按路线图执行）

**Phase A: 安全债务清理（P0，预计 1~2 天）**
- A1 lru — Cargo.lock 已为 0.16.4，警告已自动消除 ✅
- A2 paste — 路径：`netlink-packet-core` 0.8.1 → `netdev` 0.42.0。尝试升级 `netdev`
- A3 instant + A4 fxhash — 共同路径：`sled` 0.34.7 → `parking_lot` 0.11.2。评估 `sled` 升级或记录为接受债务

**Phase B: 72h Stress Test 基础设施（P1，预计 3~5 天）**
- B1 测试方案设计：双 `TestNode` 实例，5min 文件注入 + 30min 网络断开 + 2h config reload
- B2 基础设施：`cmd/syncthing/src/bin/stress_test.rs` 或 `tests/stress_72h.rs`，CSV 日志输出
- B3 验收标准：72h 无 panic、RSS 增长 < 50%、文件最终一致、重连成功率 > 95%

**Phase C: API & 兼容性补全（P2，预计 2~3 天）**
- C1 L3-APIW sub-gaps：`device` pause/resume body、`sub` 参数、override/revert 从 stub 实现
- C2 Delta Index 验证（P3，按需启动）

## 已结项（本轮完成）

- ✅ Phase E 架构债务清理：`rest.rs` + `manager.rs` 拆分，dead-code 警告消除
- ✅ `cargo clippy --all-targets`：workspace 0 warnings
- ✅ `cargo test --workspace`：294 passed, 0 failed, 3 ignored

## 未来冻结项（明确不投入）

- ❄️ BEP 扩展 `Verify` 消息族、跨实例共识、信誉系统、加密信道重建 —— 投入产出比过低，待多实例生产验证后解冻
- ❄️ QUIC Transport、MagicSocket 抽象 —— 等 TCP+Relay 路径完全稳定后评估
- ❄️ WebUI / GUI —— TUI 已覆盖 90% 核心操作，若未来确有需求基于 REST API 独立开发

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
