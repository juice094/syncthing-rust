# 后续任务清单 — 2026-05-13

> 承接 [`NEXT_STEPS_2026-05-12.md`](./NEXT_STEPS_2026-05-12.md)（已归档）  
> 本文档为 v0.2.4 发布后的活动计划，按时间窗与优先级双维度组织。  
> **状态快照（2026-05-13 晚盘）**：Phase A 完成 **6/6**；CI 全绿；HEAD = `66589c2`

---

## 0. 现状快照（2026-05-13 晚盘更新）

### 运行态
- **72h 压测**：⚠️ **意外终止**（系统休眠导致），T+9h11m，最后心跳 #1103
- **详情**：[`STRESS_TEST_PARTIAL_2026-05-12_to_05-13.md`](../reports/STRESS_TEST_PARTIAL_2026-05-12_to_05-13.md)
- **结论**：T-F1 修复在 9h+（原死亡点的 184 倍）稳定运行，但 72h 目标需在 Linux 平台重跑

### 工程态
- **HEAD**：`66589c2 refactor(T1.5): 抽取传输层 trait 到 traits/transport.rs`
- **CI**：windows + ubuntu 全绿；6 jobs 通过
- **质量门**：295 tests 全通过；`clippy::await_holding_lock` + `manual_let_else` 启用；0 警告
- **代码规模**：37 788 行 Rust；top 5 文件 = 684/606/599/569/568（service / session / connection / stun / tls）

### 已交付（v0.2.3 → v0.2.4，20 commits）
T-F1 死锁、T-F2 unwrap 审计、T-A1 baseline、T-B1 rayon 验证、T-E1 8 文件拆分、T-G2 bench-smoke CI、redundant_clone / or_fun_call / await_holding_lock。详见 `SESSION_SUMMARY_2026-05-12.md`。

### Phase A 完成 6/6（2026-05-13）
- ✅ T1.1 dialer.rs split（621 → 452 + tests 168）— commit `90e6d3b`
- ✅ T1.4 manual_let_else 全工程（10 处）— commit `921cf0f`
- ✅ T1.6 block_cache.rs split（556 → 322 + tests 234）— commit `5fd42b0`
- ✅ T1.2 types/folder.rs extraction（639 → 477）— commit `f21c8fe`
- ✅ T1.3 daemon_runner.rs split（596 → 476 + session_logger 88 + index_dispatcher 88）— commit `0140851`
- ✅ T1.5 traits/transport.rs extraction（574 → 463 + transport 127）— commit `66589c2`

---

## 1. 任务分级与排程

时间维度按"72h 压测窗口"切分：

```
[今天 2026-05-13]                [测试完成 ~05-15]              [v0.3.0 启动]
        │  压测进行中             │  压测落幕                    │
        ▼                         ▼                              ▼
   T1 短期窗口            T2 收尾 + 报告 + 滚动              T3 v0.3.0 立项
   (并行不打断压测)        (压测后即刻执行)                    (规划与设计)
```

### T1 — 压测窗口期（**完成 6/6** ✅）

不依赖运行时；**不允许**触碰 stress_test、syncthing-net 的 manager / connection；
绑定为"纯重构 / 纯文档 / 纯 CI"工作流，零运行时风险。

| ID | 任务 | 预计 | 风险 | 依赖 | 状态 |
|----|------|------|------|------|------|
| **T1.1** | `dialer.rs` tests 抽取到 `dialer/tests.rs` | 1h | 低 | 无 | ✅ `90e6d3b` |
| **T1.2** | `types/folder.rs` 抽取（FolderType / FolderStatus / Compression / Folder） | 2h | 中（pub use 复用面） | 无 | ✅ `f21c8fe` |
| **T1.3** | `daemon_runner.rs` (596) 拆分（session_logger + index_dispatcher 两个 helper） | 2h | 中 | 无（TUI 与压测无关） | ✅ `0140851` |
| **T1.4** | clippy nursery 补丁（`manual_let_else` 10 处） | 1h | 低 | 无 | ✅ `921cf0f` |
| **T1.5** | `traits.rs` (574) 抽取 transport 子模块 | 2h | 中 | 无 | ✅ `66589c2` |
| **T1.6** | `block_cache/mod.rs` (556) tests 抽取 | 0.5h | 低 | 无 | ✅ `5fd42b0` |

**T1 完成**：6/6（约 8.5h 实测）

#### T1.5 后续可选（v0.3.0 内继续）
- T1.5b：抽取 `traits/storage.rs`（FileSystem + BlockStore + ConfigStore + FolderDatabase）— ~150 行
- T1.5c：抽取 `traits/sync_model.rs`（Discovery + Events + SyncModel）— ~190 行
- 之后 mod.rs 仅保留 BepConnection + ConnectionManager（~165 行）

### T2 — 压测窗口后（~05-15 起）

| ID | 任务 | 预计 | 风险 | 触发 |
|----|------|------|------|------|
| **T2.1** | `72H_STRESS_REPORT_2026-05-15.md` 撰写（内存曲线、连接数、错误率、监控 CSV 解析） | 3h | 低 | 压测落幕 |
| **T2.2** | T-F3 `tracing-appender` 日志滚动（hourly / daily rotation） | 2h | 低 | T2.1 完成 |
| **T2.3** | `service/mod.rs` (684) 业务拆分 — 需要先出**架构 RFC** | 1h RFC + 4h impl | 高 | T2.2 完成 |
| **T2.4** | Linux 平台重跑 72h 压测（验证 epoll/io_uring 路径无新死锁） | 后台 72h | 中 | T2.1 完成 |

### T3 — v0.3.0 立项窗口（~05-16 起）

| ID | 任务 | 预计 | 风险 | 触发 |
|----|------|------|------|------|
| **T3.1** | T-D3 `pending_responses` slab 化 | 3h | 中 | T2 全部完成 |
| **T3.2** | T-C `FileSystemDatabase` slab + sled WAL 设计 RFC | 1d | 高 | T2 完成 |
| **T3.3** | v0.3.0 milestone 创建 + scope 定稿（GitHub Issue tracker） | 0.5d | 低 | T2.1 + T2.4 |
| **T3.4** | dialer.rs 业务拆分（dial / score / retry，依赖 T1.1） | 3h | 中 | T1.1 完成 |

---

## 2. 推荐执行编排

### 阶段 A — 今日（剩余约 14h 工作窗口）
1. **T1.1**：dialer.rs tests 抽取（~1h，确定性高，立刻动）→ 1 commit
2. **T1.4**：clippy nursery（~1h）→ 1 commit
3. **T1.6**：block_cache tests 抽取（~0.5h）→ 1 commit
4. **T1.2**：types/folder.rs 抽取（~2h，需 pub use 仔细处理）→ 1 commit
5. 末次 push，等 CI 全绿后入睡。

### 阶段 B — 明天 2026-05-14
1. **T1.3**：daemon_runner.rs 三段拆分
2. **T1.5**：traits.rs 按 trait 分组拆分
3. 期间监测 `stress-heartbeat.log`，若心跳停滞超过 5 分钟立刻 dump 堆栈。

### 阶段 C — 2026-05-15 压测落幕
1. 立刻执行 **T2.1** 报告撰写（基于 csv + log）
2. 同日启动 **T2.2** log rotation 改造
3. 当天即跑 **T2.4** Linux 版（需切 WSL 或远程 host）

### 阶段 D — 2026-05-16 起
1. **T2.3** 撰写 service RFC
2. **T3.3** 起 v0.3.0 milestone
3. 视 RFC 反馈推进 **T2.3** impl

---

## 3. 风险与回退

| 风险 | 触发条件 | 应对 |
|------|----------|------|
| 压测意外冻结 | 心跳 >5min 无新行 | 立刻 dump 进程堆栈（procdump / windbg），抓 trace |
| 重构破坏 CI | clippy 或 fmt 失败 | 单 commit 回滚 + `git revert` |
| service 拆分卡壳 | RFC 评审无定论 | 接受当前 684 行，转向 T3.1/T3.2 |
| Linux 压测发现新死锁 | T2.4 心跳停滞 | 同 T-F1 流程：捕获 → RCA → fix → 重跑 |
| v0.3.0 范围膨胀 | T-C 设计变复杂 | 拆 0.3.0（轻量）+ 0.4.0（持久化层重构） |

---

## 4. 工程纪律（保持继承）

每次 commit 必须满足：
- ✅ `cargo fmt --check` 通过
- ✅ `cargo clippy --release --all-targets -- -D warnings -W clippy::await_holding_lock` 通过
- ✅ `cargo test --workspace --lib` 295/295 通过
- ✅ CI 双平台绿

每次 v0.X.Y release 必须：
- ✅ 所有 Cargo.toml 版本一致
- ✅ CHANGELOG 更新
- ✅ release notes（含关联 RCA / baseline / 报告）
- ✅ git tag 注解

新增（自 2026-05-13 起）：
- ✅ 大文件拆分 commit 必须含 `wc -l before/after` 的注释
- ✅ 业务模块拆分必须先有 RFC（drafts/RFC-XXX.md）

---

## 5. v0.3.0 候选范围（初稿，可调整）

按价值/工作量排序：

1. **T2.3** service/mod.rs 拆分 → 中价值 / 中工作量
2. **T2.2 + T2.4** 长跑可靠性（log rotation + Linux 验证）→ 高价值 / 低-中工作量
3. **T3.1** pending_responses slab → 低价值 / 中工作量
4. **T3.2** FileSystemDatabase WAL → 高价值 / 高工作量（可能延到 v0.4.0）
5. **dialer 业务拆分**（T3.4）→ 中价值 / 中工作量
6. 跨版本互通基线（与 Go syncthing 2.x 长程互通 72h 测试）→ 高价值

**建议 v0.3.0 定义**：
- 必含：T2.2、T2.4、T1.1~T1.6 全部、T3.4
- 选含：T2.3、T3.1
- 推后到 v0.4.0：T3.2

---

## 6. 跨文件跳转速查

- **当前权威路线图**：本文件
- **2026-05-12 单日归档**：[`SESSION_SUMMARY_2026-05-12.md`](../reports/SESSION_SUMMARY_2026-05-12.md)
- **历史路线图**：[`POST_V0_2_0_ROADMAP.md`](./POST_V0_2_0_ROADMAP.md)（仍部分有效，特别是 P3/P4/P5 战略项）
- **持锁审计**：[`LOCK_AWAIT_AUDIT_2026-05-12.md`](../reports/LOCK_AWAIT_AUDIT_2026-05-12.md)
- **死锁 RCA**：[`STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md`](../reports/STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md)
- **Baseline 指标**：[`BASELINE_2026-05-12.md`](../reports/BASELINE_2026-05-12.md)
- **unwrap 审计**：[`UNWRAP_AUDIT_2026-05-12.md`](../reports/UNWRAP_AUDIT_2026-05-12.md)
- **横向调优母计划**：[`TUNING_PLAN_2026-05-11.md`](./TUNING_PLAN_2026-05-11.md)

---

**维护人**：juice094  
**下次刷新**：T2.1 报告完成时（预计 2026-05-15）
