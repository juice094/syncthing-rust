---
type: report
title: 2026-10-16 工作交付报告
description: CI 自长期红恢复全绿；三处产品缺陷修复（.stignore watcher 路径 / 在途拉取误判删除 / 拉取失败不重试）；§1 会话竞态根治并关闭 §24 CI 覆盖缺口；MCP 工具集 11→19；顶层文档生产化重构与 skill/DSH 集成面评估。
tags: [report, handoff, ci, e2e, cluster-config, mcp, docs, okf]
status: active
project: syncthing-rust
timestamp: 2026-10-16T00:00:00Z
---

# 工作交付报告 · 2026-10-16

> 面向后续维护与开发的交接文档。覆盖：本日全部代码 / 文档 / CI 交付、三处产品缺陷与一处测试基建根因、当前系统状态与 backlog。
> 所有结论均取自实测（本地五道门、`gh run list`、带 `RUST_LOG` 的复现日志），未使用推测。

---

## 一、交付时间线（均为 juice094 身份，已推送 origin/main）

| Commit | 内容 |
|:---|:---|
| `51da17d` | **修复 dependabot 大版本升级引起的编译中断**（rand 0.8→0.10、sysinfo 0.30→0.39、ctor 0.2→1.0、tokio-tungstenite 0.30、tower 0.5 等） |
| `55aaa5b` | **CI 自 2026-06-29 起的长期红转绿**：Cargo Deny 改用 `taiki-e/install-action`；Security Audit 补 `checks: write` |
| `f99f0ac` | 审计债务刷新：`resvg` 关 text 特性（去 `ttf-parser`/`rustybuzz`）、去 `paste`、新增 `h2` 条目 |
| `dce379b` `b0227b7` `b305359` | e2e_crud 加固：会话按**连接粒度**断开、块请求 3×重试且每次重取连接、IndexUpdate 退避重发 |
| `8006088` | **产品修复：`.stignore` 目录规则未作用于 watcher 增量路径** |
| `85190ae` | **产品修复：在途拉取被子树扫描误判删除** + 拉取失败退避重试 + 回归测试 |
| `6aaa6aa` `3c4405b` | 就绪等待原语 + **单边拨号根治 §1 会话竞态**；e2e_sync / e2e_crud 的 `#[ignore]` 解除 |
| `f4b3cb5` | **MCP 工具集 11 → 19** + 真实二进制集成测试 |
| `5425be7` `fb50f6c` `b9d4da5` `3e48e13` `a33af01` | 文档生产化重构、agent bundle 刷新、KNOWN_ISSUES §1/§23/§24 登记、事实源收敛 |
| `8a95870` | `scripts/agent-facts.ps1`（只读事实导出） |

（另有 `c5ebc7a`/`a941226` 两条 KNOWN_ISSUES §23 根因登记。）

---

## 二、产品缺陷修复（三处，均带回归测试或实测证据）

1. **`.stignore` 目录规则不作用于 watcher 增量路径**（正确性 / 隐私）
   `IgnoreMatcher::matches` 对 `dir/` 规则仅在 `is_dir=true` 时生效，而 `scan_changed_file` 传的是**文件**路径 →
   被忽略目录内的改动仍被索引并同步给对端。修复为新增 `is_ignored_with_ancestors()`（逐级检查祖先），
   用于 `scan_changed_file` 与 sub-scan 根。**实测证据**：修复前接收端日志出现 `ignored/secret.txt` 被拉取。
2. **在途拉取被子树扫描误判删除**（数据安全相邻）
   远程索引先写 DB、块传输后落盘；此窗口内 watcher 触发的子树扫描按「DB 有记录但磁盘无文件」推断删除，
   以**新版本向量**广播 → 源端版本更低，把自己唯一副本移入 `.sttrash`，文件从此再也同步不过去。
   修复：`scan_incremental()` 在读到的既存状态为 `Pulling` 或 `pending_pulls` 非空时**关闭子树删除推断**
   （显式删除事件与空闲态扫描不受影响）。**实测证据**：`e2e_crud` `kept/public.txt` 完整链路日志；
   修复后该用例 6 次套件运行 0 次失败（修复前 2 次失败）。
3. **拉取失败不重试**（可靠性）
   `Puller::pull_folder` 只统计失败数，失败文件要等下一个索引事件。修复为 `start_pull_loop` 在有失败时按
   `PULL_RETRY_BACKOFF_INIT_MS`(500ms) 起逐次翻倍、封顶 `PULL_RETRY_BACKOFF_MAX_MS`（= 原 10s 周期）提前重试，
   成功即归零；正确性仍由 `check_needed_files()` 从 DB/FS 差异重新推导。

---

## 三、测试基建与 CI

### 3.1 §1 ClusterConfig 竞态根治（本轮最大收益）

- **根因（已验证）**：`registry.rs` 的连接竞争解决在双方是**对称且正确**的（四种注册顺序组合都收敛到同一条物理连接，
  未改动它）。真凶是**双方同时拨号**：两端各产生 incoming + outgoing，失败方那条连接上的 `BepSession` 坐在
  「本端 open、对端无人读」的链路上，直到 10s ClusterConfig 超时才退出 —— 每个用例白等 10s，再与块请求重试叠加冲破断言窗口。
- **修复**：① 测试全部改**单边拨号**（A `connect_to(B)`，B 只 `add_peer_device(A)`，新增该 API）；
  ② `bep_bridge` 会话槽位与连接在 `on_connected` 内**同步认领**、会话槽位改存 `AbortHandle`，
  看护任务保证**被 abort 时也按 `conn_id` 断开自己那条**。
- **验收（按「零告警」而非只看测试绿）**：

| 指标 | 修复前 | 修复后 |
|:---|:---|:---|
| `WARN Timeout waiting for ClusterConfig` | 每轮必现 | **0 条** |
| `e2e_crud` | 26–85s，9 次套件运行 4 次失败 | **6/6 全绿，2.2–3.1s/次** |
| `e2e_sync` | 被 `#[ignore]`，需 90s 预算 + 30s 断连重拨 hack | **3/3 通过，0.14–0.36s/次** |

### 3.2 §24 CI 端到端覆盖缺口关闭

`e2e-test` job 此前只跑 `e2e_sync`，而它唯一用例被 `#[ignore]` → **实际执行 0 条断言**。
现在改为**串行执行 `e2e_sync` + `e2e_crud`**（`timeout-minutes` 5 → 15），首次运行即通过（3m34s）。

### 3.3 CI 恢复与审计债务

Cargo Deny（`taiki-e/install-action`）与 Security Audit（`checks: write`）修复后 CI 转绿；
审计债务收敛为 3 项上游 unmaintained 传递依赖（`instant`/`fxhash`/`h2`），`deny.toml` 与 `.cargo/audit.toml` 同步。

---

## 四、文档与 Agent 集成面

- **顶层文档生产化重构**：`AGENTS.md` 425 行 → 95 行入口页（只留红线 + 导航），`CLAUDE.md` 同理，正文约束下沉 `docs/agent/`。
- **消除事实漂移**：全仓手抄的测试基线全部移除（README 徽章、CONTRIBUTING 自相矛盾、AGENTS/testing/claude_reference/roadmap 等），
  数值唯一事实源收敛到 [`docs/agent/testing.md`](../agent/testing.md)；新增 `scripts/agent-facts.ps1` 只读导出（版本/端口/MCP 工具数/CI job/超 600 行文件）。
- **MCP**：工具 11 → 19（新增 health/metrics/db-status/browse/file/pause/resume/poll_events），
  支持 `SYNCTHING_API_KEY` 转发、查询值百分号编码；名册拆出 `src/tools.rs`（守 600 行软上限）；
  `tests/mcp_bridge.rs` 用真实二进制 + 进程内 stub HTTP 服务端（零新依赖）。
- **skill**：`.agents/skills/bep-protocol/SKILL.md` 原描述停留在 v0.2.8 Alpha，已重写为指向权威文档的操作入口。
- **DSH**：评估结论落 [`docs/design/AGENT_INTEGRATION.md`](../design/AGENT_INTEGRATION.md) —— 能力已就绪、**落点在 Gray-Harness 仓**
  （`packages/gray-syncthing`，host 半边代理 REST、client 只注册 slot），本仓未改代码。

---

## 五、当前系统状态与验证

| 维度 | 状态 |
|:---|:---|
| 工作区 / 同步 | 干净；`HEAD == origin/main == a33af01` |
| 测试基线 | **454 passed / 5 ignored / 0 failed**（38 个测试二进制，2026-10-16 实测） |
| 五道门 | fmt / clippy(0 warn) / test / doc / `cargo deny check all` 全绿 |
| CI | `CI` 17/17 job + `Build Release Binary` 3/3 job 均 success @ `a33af01` |
| 忽略用例 | 5 个：UDP loopback broadcast（CI runner 不稳）、需外网的 relay/STUN 用例与 2 个 STUN doc-test |
| worktree | 仅主工作树（对照用的 `sr-base` 已移入回收站，失联元数据已 prune） |

复现命令：`pwsh scripts/agent-facts.ps1`（事实导出）、`cargo test --workspace`、`gh run list --limit 3`。

---

## 六、残留与 Backlog

1. **回收站仍存 `sr-base` 目录**：按「删除优先回收站」规则未硬删，磁盘需清空回收站后才真正释放。
2. **历史记录中的测试数值**：`CHANGELOG` v3.0.3 条目、`docs/reports/SESSION_REPORT_2026-07-21.md` 为发布史/报告快照，有意保留；
   `RELEASES.md` 已加顶部说明并移除唯一的「current workspace」表述，统一指向 [`docs/agent/testing.md`](../agent/testing.md) 事实源。
3. **后置的两条产品侧方向（§23 已登记）**：`BepSession` 在 ClusterConfig 超时后自行退避重试；
   `ManagerBlockSource` 选连接时不校验该连接上是否有活跃会话。测试基建修复后二者已无直接危害。
4. **本仓无 lefthook 配置**：全局 `core.hooksPath=~/.git-hooks` 的 lefthook 会因缺配置直接失败，
   提交需 `$env:LEFTHOOK='0'`（`commit-msg` 的 Conventional Commits 校验仍每次执行）。
5. **v3.1.0 准入线**：72h 耐久测试仍未跑（此前受 §3 环境限制：Windows 桌面休眠会回收 nohup 子进程）。
6. **DSH 插件落地**（Gray-Harness 仓工作面）：`packages/gray-syncthing`（host 半边优先、只读能力先行 → 进门禁与名册 → 挂载层变更需先备份 `~/.dsh`）。

---

← 返回 [docs/README.md](../README.md)
