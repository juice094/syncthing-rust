# Agent 环境指引 · syncthing-rust

> 📋 **当前权威路线图**: [`docs/plans/POST_V0_2_0_ROADMAP.md`](./docs/plans/POST_V0_2_0_ROADMAP.md)
> 📋 **Better-Than-Go 主计划**: `~/.claude/plans/generic-forging-matsumoto.md`
> 📋 **计划索引**: [`docs/plans/INDEX.md`](./docs/plans/INDEX.md)
> 📋 **审计报告**: [`docs/plans/PLAN_AUDIT_2026-04-27.md`](./docs/plans/PLAN_AUDIT_2026-04-27.md)

## 项目定位

`syncthing-rust` 是 P2P 文件同步的 Rust 替代实现。长期目标：基于 Rust 的内存安全、零成本抽象、类型安全协议、库可嵌入性，成为优于 Go 版的 syncthing。

- **当前状态**：v3.0.2 Production，**347 passed / 4 ignored / 0 failed**，**0 warnings**，**CI 全绿 (19/19)**
- **传输层**：TCP+TLS / HTTP CONNECT 代理 / SOCKS5 代理 / DERP 中继（自研协议）/ UPnP / **Relay v1 并行拨号 ✅**
- **发现层**：Local Discovery（UDP 广播骨架）⚠️ / STUN / PortMapper / **Global Discovery ✅** / **Relay Protocol v1 ✅**
- **同步**：Pull ✅ / Push（主动 IndexUpdate 推送）✅ / 双向同步已实测验证 ✅
- **版本控制**：Simple ✅ / Staggered ✅（4 时间窗口, `.stversions/` 归档）
- **协议**：Hello→prost ✅ / LZ4 写入压缩 ✅ / ClusterConfig ✅ / Ping/Pong ✅ / wire_compat 10 tests ✅
- **传输**：TCP keepalive (60s/10s) ✅ / 连接重试累加 ✅ / **BEP Relay v1 ✅**（10 relay 候选 + 直连竞速）
- **互操作**：Rust↔Rust 双向 E2E ✅；Go Syncthing 互操作已验证 ✅
- **观测**：REST API 读写端点（兼容 Go 布局）+ 文件系统 watcher(1s debounce) + **TUI 实时状态（event bridge）✅** + **配置热重载 ✅**
- **运维**：systemd service ✅ / cloud-deploy.sh ✅ / WSL 交叉编译 ✅ / git bundle 灾备同步 ✅

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

## 阶段性进展（截至 2026-06-07 会话末）

### 2026-06-07 (S2): §15 Root Cause Fix + Text Merge Conflict Resolution  

| 模块 | 内容 | 状态 |
|------|------|------|
| §15 Root Cause Fix | `index_handler` 忽略未知文件的远程删除标记，消除级联删除 | ✅ (`af229d5`) |
| Gray-Desktop Deploy | 新格雷节点 (100.69.11.71, IYGOGGD-...) 配置同步上线 | ✅ |
| Syncthing Status | ROG-X ↔ Gray BEP 双向同步正常 (52 files exchanged, 0 failed) | ✅ |
| DB Reset Protocol | Gray 格式化后双端 DB 清空 + 重新索引验证通过 | ✅ |
| Text File Merge | `crates/syncthing-sync/src/merge.rs`: 文本文件三路合并 | ✅ (`5bf3fe8`) |
| Conflict Resolution | 可合并文本自动合并；重叠修改插入 git 风格冲突标记；二进制回退 RenameBoth | ✅ |
| Dependencies | 新增 `similar = "2.6"` 用于行级 diff | ✅ |
| CI | 扣减后全绿 (clippy, test, deny, fmt) | ✅ |

### 2026-06-07: v3.0.0 Release — Production-grade P2P File Sync

| 模块 | 内容 | 状态 |
|------|------|------|
| CI Full Green | 19/19 jobs passing: clippy (10 warnings fixed), cargo-deny (deny.toml), bench compile, formatting | ✅ |
| Cargo Deny Config | License audit allowlist + advisory ignores (paste, instant, fxhash via sled) | ✅ |
| deny.toml CI Compat | 移除 `allow-workspace` 键以兼容 CI 旧版 cargo-deny 二进制 | ✅ |
| Version Bump | 14 crates 0.2.x → 3.0.0 | ✅ |

### 2026-06-02/03: Phase 0 Foundation Hardening

| 模块 | 内容 | 状态 |
|------|------|------|
| P0.1 BEP Protocol | Hello→prost derive (-180行手写); LZ4 写入压缩 | ✅ |
| P0.2 Connection Lifecycle | ClusterConfig/Ping/Close/DeviceID 验证 — 已有实现，审计确认 | ✅ |
| P0.3 Wire API | pause/resume/scan/override/revert — SyncService trait impl 已全部实现 | ✅ |
| P0.4 Ignore Patterns | `**/` 任意深度匹配; `//`→`#` 注释兼容; `#include` 修复 | ✅ |
| rename_with_retry | Windows 文件锁回退（remove→rename→指数退避） | ✅ |
| Scanner 日志增强 | scanned/new/modified/changed 计数器 | ✅ |
| Pull loop 修复 | MIN_PULL_GAP=2s 合并远程索引通知; 日志噪音降低 99% | ✅ |
| E2E 双向同步 | gray-workspace 574 文件→云端; 1秒内检测+推送新文件 | ✅ |

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
| 组件分发拆分 | `syncthing-bench` / `syncthing-cli` 从 `cmd/syncthing` 提取为独立二进制 | POST_V0_2_0 | ✅ |
| 解耦工作 | 元数据统一、`test_harness` 归位、E2E 测试外迁、`syncthing-net` API 收敛 | *自发* | ✅ |

### 2026-06-04: 生产部署 — ROG-X ↔ Gray-Cloud 双端工作区同步

| 模块 | 内容 | 状态 |
|------|------|------|
| Cloud Rebuild | 云端格式化 → Tailscale IP 变更 (100.127.13.26→100.113.140.121) | ✅ |
| WSL Cross-Compile | WSL Ubuntu 内 `cargo build --release` 产出 Linux ELF (14MB, 5m48s) | ✅ |
| SCP Deploy | 二进制上传 `/usr/local/bin/syncthing` + systemd service | ✅ |
| OpenClaw Workspace Sync | `C:\Users\22414\.kimi_openclaw\workspace` ↔ `/root/.openclaw/workspace` BEP 双向同步 | ✅ |
| DB Reset Protocol | 旧 DB 残留索引导致 2043 文件被误删，确立对侧格式化后必须双端重置 DB 的操作流程 | ✅ |
| Git Bundle 灾备 | `git bundle` → SCP → cloud `git clone` 恢复权威 workspace 快照 | ✅ |
| Stale Index Bug | 发现并登记 §15：对侧重装后 DB 残留触发 NoSuchFile 风暴 + puller 误删本地文件 | ✅ |
| **Sequence Race Fix** | `FileSystemDatabase::increment_sequence` read-modify-write 竞态 → `seq_locks` per-folder Mutex (341 passed) | ✅ |
| .stignore Sync Gap | `.stignore` 自身被排除规则排除，需手动 SCP 部署（已补充到灾备协议） | ✅ |
| syncthing-ops Skill | 灾备恢复/异常诊断/新端部署/git 共存 四场景 SOP → `workspace/skills/registered/syncthing-ops/` | ✅ |

### 2026-06-03 (续): Phase 0.5 + Phase 1.1

| 模块 | 内容 | 状态 |
|------|------|------|
| Step 1: push fix | index_handler 本地独有文件推送 | ✅ |
| Step 2: rename verify | 单元测试 + 代码部署 | ✅ |
| Step 3: P0.6 Cross-Impl | `wire_compat.rs` 10 协议一致性测试 | ✅ |
| Step 4: Simple Versioning | `syncthing-versioner` crate + puller 集成 + 379 tests | ✅ |
| Pull loop 回退 | 还原原始逻辑（E2E 兼容），日志已降 DEBUG | ✅ |
| Watcher fix | `.syncthing.*.tmp` 过滤 + 5s debounce + 5s gap | ✅ |

### 未完成（计划内阻塞项）

| 模块 | 内容 | 来源计划 | 状态 | 优先级 |
|------|------|----------|------|--------|
| rename_with_retry 真实验证 | Kimi Claw Desktop 独占锁场景 | P0 修复 | 🟡 | P1 — VS Code 共享读不触发 |
| 72h Stress Test | 长期运行稳定性验证 | PHASE3_PLAN 3.4 | ⏳ | **P0** |
| Fix 1: 连接重试累加 | retry_count 独立 map, 成功后清除 | production-readiness | ✅ | P1 |
| Fix 2: TCP keepalive | SO_KEEPALIVE 60s/10s/3probes | production-readiness | ✅ | P1 |
| Fix 3: BEP Relay v1 激活 | 5 行顺序 bug 修复 + init wizard 默认启用 | production-readiness | ✅ | P1 |
| Fix 4: Staggered 版本 | 4 时间窗口, maxAge 可配 | Better-Than-Go P1.1b | ✅ | P1 |
| P1.2 Symlink 同步 | scanner + puller + 平台守卫 | Better-Than-Go | 🔲 | P1 |
| P1.4 ReceiveOnly 语义 | BEP local flags | Better-Than-Go | 🔲 | P1 |
| Delta Index 验证 | IndexID + Sequence 一致性 | PHASE4_PLAN 4.2 | ⏳ | P3 |
| PCP/NAT-PMP | PortMapper UPnP fallback | POST_V0_2_0 Phase D | ⏳ | P3 |

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
- ✅ **本地 30min 短周期预验证通过（2026-04-27）**: 0 errors, 0 panics, 连接保持稳定
- 下一步：移交格雷远程执行 72h 完整测试

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
- ✅ `cargo test --workspace`：309 passed, 0 failed, 4 ignored

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

- `daemon_runner.rs` 当前 421 行，**禁止继续膨胀**。新增网络组件（如 DERP、WebSocket proxy）时，必须拆分为独立模块（如 `discovery_task.rs`、`relay_task.rs`、`dial_task.rs`）。
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

---

## Agent Skill 注册：syncthing-rust 双节点测试协调

> **Skill ID**：`skill-syncthing-dual-node-test`  
> **生效版本**：v0.2.7+  
> **注册时间**：2026-05-15  
> **对侧节点**：Gray-Cloud (Ubuntu 24.04 VPS, Tailscale 100.113.140.121, systemd `syncthing.service`)  
> **本侧节点**：Windows 11 (Tailscale 100.107.247.38)

### 触发条件

当用户消息包含以下关键词时，本 Skill 激活：
- "格雷" / "Gray-Cloud" / "对侧" / "远端"
- "双节点" / "真实网络" / "E2E" / "端到端"
- "压测" / "压力测试" / "耐久" / "72h" / "stress"
- "编译二进制" / "推送" / "发布 Release"
- "接收资料" / "日志分析" / "排查"

### 本侧能力边界

| 能力 | 状态 | 说明 |
|------|------|------|
| 编译 Windows 二进制 | ✅ | `cargo build --release` → `target/release/syncthing.exe` (~12.6MB) |
| 编译 Linux 二进制 | ✅ | WSL2 `cargo build --release` → `target/release/syncthing` (~13.6MB) |
| 生成交互式配置 | ✅ | `syncthing.exe init` wizard，生成 config.json + cert.pem + key.pem |
| 启动/停止/重启节点 | ✅ | `Start-Process` / `Stop-Process`，指定 `--config-dir` |
| 监控连接状态 | ✅ | `netstat -ano` + `tasklist` + `Get-NetTCPConnection` |
| 生成操作手册/报告 | ✅ | Markdown 报告，base64 编码写入 Windows 文件系统 |
| 接收格雷侧日志分析 | ⚠️ 间接 | 用户转发日志文本，本机进行 grep/模式分析 |
| 直接 SSH/SCP 到格雷 | ❌ | 无网络直达能力，必须通过用户中转 |

### 标准操作流程（SOP）

```
Step 1: 版本对齐
    └─ 确认双方使用同一 Git tag（如 v0.2.7），若不一致则重新编译推送

Step 2: 生成本侧配置
    └─ 运行 `syncthing.exe init --config-dir <dir>`
    └─ 输入：设备名、同步路径（必须为 config-dir 子目录，如 `<dir>\sync`）、对侧 Device ID、对侧地址
    └─ 修改 folder ID 匹配对侧（如 `test-folder`）
    └─ 提取本侧 Device ID（以 cert.pem 为准，非 wizard 打印值）

Step 3: 启动本侧节点
    └─ `Start-Process syncthing.exe -ArgumentList "run","--config-dir",...`
    └─ 确认 PID 存在、端口 22001/8385 监听

Step 4: 与格雷侧交换信息
    └─ 向用户发送：本侧 Device ID、Tailscale IP、监听端口、Folder ID
    └─ 从用户接收：格雷侧 Device ID、Tailscale IP、Folder ID

Step 5: 等待格雷侧配置完成
    └─ 格雷侧更新 config.json 中的 devices + folders.devices
    └─ 格雷侧启动守护进程

Step 6: 验证连接建立
    └─ `netstat -ano | grep 100.113.140.121`
    └─ 目标：ESTABLISHED 稳定保持 20s+
    └─ 若 SYN_SENT 挂死 → 检查 Tailscale / 防火墙 / 对侧监听地址

Step 7: 执行测试任务
    └─ 在 sync 目录放置测试文件，观察双向同步
    └─ 根据 GRAY_CLOUD_OPS_MANUAL_v0.2.7.md 执行 Task 2~5

Step 8: 收集结果与报告
    └─ 验证文件内容一致性
    └─ 更新 `docs/reports/DUAL_NODE_TEST_*.md`
    └─ git commit + push
```

### 灾备恢复协议（对侧格式化/重装后）

1. 停止双端 syncthing
2. 删除双端 `db/` 和 `syncthing.pid`
3. 本侧 `git bundle create workspace.bundle --all`
4. SCP → 对侧 `git clone workspace.bundle workspace`
5. 对侧 `systemctl start syncthing`
6. 本侧启动 syncthing
7. 验证：0 error code 3 + scans stable (files_changed=0)

### 待测试任务清单（P0 → P2）

| 优先级 | 任务 | 触发条件 | 验收标准 |
|--------|------|----------|----------|
| **P0** | 72h 耐久测试 | 用户指令"开始耐久测试" | 72h 无 panic、RSS 增长 < 50%、文件最终一致、重连成功率 > 95% |
| **P0** | 大文件压测 | 用户指令"压测大文件" | 10MB/100MB/500MB 文件双向同步成功，无 Block 丢失 |
| **P1** | 网络抖动/断线重连 | 用户指令"测试断线重连" | 10 次模拟断线，连接恢复率 100%，Index 续传正确 |
| **P1** | 内存泄漏监控 | 耐久测试并行执行 | 每 5 分钟记录 RSS，无持续增长趋势 |
| **P2** | 元数据排除验证 | Task 1 部署 `.stignore` 后 | `db.syncthing.tmp`、`config.syncthing.tmp` 等不再出现 |

### 与格雷侧协作通信协议

**信息交换格式**（用户中转）：

```markdown
**格雷 → 宿**
- 守护进程 PID：`<pid>`
- Device ID：`<device-id>`
- 监听地址：`0.0.0.0:22001`
- Folder ID：`test-folder`
- 日志片段：```...```

**宿 → 格雷**
- 守护进程 PID：`<pid>`
- Device ID：`<device-id>`（以证书为准）
- Tailscale IP：`100.107.247.38`
- 同步目录：`C:\...\sync`
- 操作手册：`docs/GRAY_CLOUD_OPS_MANUAL_v0.2.7.md`
```

### 已知限制与风险

| 限制 | 影响 | 规避方法 |
|------|------|---------|
| WSL2 `/mnt/c` 挂载不稳定 | Bash 无法直接 `ls/cp` Windows 路径 | 全部文件操作走 PowerShell (`powershell.exe -Command ...`) |
| Bash/PowerShell 引号转义地狱 | 长内容（脚本/配置）写入失败 | 使用 base64 编码 → PowerShell `[Convert]::FromBase64String` 解码写入 |
| Windows 日志捕获困难 | `Start-Process -RedirectStandardError` 常为空 | 依赖 `netstat` + API + 文件系统检查代替日志诊断 |
| API 端点空引用异常 | `/rest/system/connections` 可能 500 | 使用 `netstat` 和 `tasklist` 替代 |
| 无法直达格雷网络 | 不能 SSH/SCP 到对侧 | 所有资料通过用户消息中转 |
| Device ID 证书陷阱 | `local_device_id` 会被 cert.pem 覆盖 | **始终以证书/API 为准确认 Device ID**，init wizard 打印值仅供参考 |

### 相关文件索引

| 文件 | 用途 |
|------|------|
| `docs/GRAY_CLOUD_OPS_MANUAL_v0.2.7.md` | 格雷侧完整操作手册（自动化脚本、API 速查、紧急上报） |
| `docs/reports/DUAL_NODE_TEST_2026-05-15.md` | 双节点测试报告（含时间线、Bug 分析、修复记录） |
| `.github/workflows/ci.yml` | CI 配置（含 e2e-test、release-check、doc-check） |
| `cmd/syncthing/src/init_wizard.rs` | 交互式配置生成向导 |
| `crates/syncthing-sync/src/scanner.rs` | Scanner 逻辑（已知缺陷：无自动元数据排除） |
| `crates/syncthing-net/src/tcp_transport.rs` | TCP 传输 + 连接状态设置 |

---

**冻结声明**：本 Skill 涉及的 SOP 和任务清单随 v0.2.7 验证通过而固化。后续版本若引入新的网络传输层（QUIC/WebSocket）或配置格式变更，需同步更新本 Skill 中的地址格式和连接检查命令。
