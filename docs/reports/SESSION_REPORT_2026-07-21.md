---
type: report
title: 2026-07-21 工作交付报告
description: 版本向量根因修复、数据安全事故加固、冲突死循环 Go 对齐、官方互联验证与 e2e 回归归因。
tags: [report, handoff, conflict, vector-clock, hardening, interop]
status: active
project: syncthing-rust
timestamp: 2026-07-21T00:00:00Z
---

# 工作交付报告 · 2026-07-21

> 面向后续维护与开发的交接文档。覆盖：本日全部代码/文档交付、两起生产事件的 RCA、与 Go 官方实现的对齐分析、当前系统状态与 backlog。

---

## 一、交付时间线（均为 juice094 身份，已推送 origin/main）

| Commit | 内容 |
|:---|:---|
| `30337a4` | TUI AddFolder 静默丢失修复 + TUI 事件层 11 个 handler 级测试 |
| `378b03a` | BEP Ping 风暴（互答循环）、block_source 时机、配置重协商钩子 |
| `83acf07` / `9b1da43` | 文档：§20/§21 登记、基线同步 |
| `8225c0f` | **版本向量真实设备计数器 ID**（硬编码 1 → 证书派生，Go ShortID 语义） |
| `0be7f19` | 删除路径三层加固：pull 删除配额 / `.sttrash` 软删除 / 删除审计日志 |
| `94e5db0` | 文档：§22 数据安全事故 RCA |
| `e2cf6fa`~`886ccee` | （下游会话）幻影删除、last_pong、僵尸会话重连、版本乒乓、.stversions 泄漏 |
| `9c9877a` | 冲突死循环分析报告与修复计划（`docs/plans/CONFLICT_RESOLUTION_ALIGNMENT_2026-07-21.md`） |
| `73b42c5` | **确定性冲突胜者仲裁**（Go WinsConflict 语义）+ 收敛证明测试 |

测试基线：**433 passed / 6 ignored / 0 failed**（单元+集成；e2e_crud 当前红色，见 §四）。

## 二、事故与根因（详细 RCA 见 KNOWN_ISSUES §20~§22）

1. **Ping 风暴**：`BepSession` 收 Ping 回 Ping，双端线速互答（16MB/16min 日志）。BEP Ping 单向无 Pong。修复后仅 30s 定时器发 Ping。
2. **10-Courses 数据丢失（2026-07-20）**：云端陈旧索引经 IndexUpdate 批次传播删除标记，114 文件被静默删除。根因链：版本向量计数器硬编码 1（§21）→ 线性误判支配 + 全量阈值不覆盖增量批次 + 硬删无兜底。三层防御已落地（配额/回收站/审计）。
3. **冲突死循环**：双端同文件独立谱系时反复回拉覆盖。根因：`resolve_conflict` 无仲裁一律取远程。修复：Go `WinsConflict` 确定性胜者（mtime → 向量并发方向 tiebreak），双端一次交换收敛。

## 三、与 Go 官方实现的对齐结论

- 冲突三零件：`InConflictWith`（内容精化，`PreviousBlocksHash`）/ `WinsConflict`（确定性胜者）/ `moveForConflict`（`.sync-conflict-时间戳-设备短ID`）。本次已实现 2/3；**Phase 2 内容精化待做**（用现有 `base_version`/`blocks_hash` 短路伪冲突）。
- 版本向量计数器 ID：Go 用设备证书哈希前 8 字节；我方 `DeviceId::counter_id()` 同语义。
- 删除设计参考：Go `ignoreDelete` 文件夹选项 + Trash Can versioning（我方 `.sttrash` 已覆盖后者）。

## 四、当前系统状态

- **e2e_crud 红色（待下游修复）**：二分归因 `886ccee`（content-hash-first change detection）。`94e5db0`/`f4fce05` 全绿，`9c9877a` 起 2/5 超时失败。疑似哈希优先检测误判新下载文件为本地变更。
- **官方互联**：syncthing-rust ↔ Go Syncthing v2.1.1 loopback 双向同步 + 1MiB 多块 SHA-256 一致（`target/interop/` 可复跑）。
- **本机生产节点**（PID 27660，新二进制）：REST 绑回 `127.0.0.1:8385`；两文件夹 Simple versioning keep=5；删除加固激活。
- **云端（Gray-Cloud）**：**仍是事故前旧二进制**，索引疑似腐败；`target/release/syncthing-linux-x86_64` 新件已备（含全部修复）。
- **10-Courses 恢复**：手机（HONOR）事故期间离线，为最佳恢复源，备份前勿上线。

## 五、Backlog（按优先级）

1. **下游修复 `886ccee`** e2e 回归（或评估 revert）——origin/main 套件红色中。
2. **云端更新**（P2-1）：先 SSH 确认 10-Courses 实体文件是否还在 → 部署新二进制 → 验证重连。
3. **冲突 Phase 2**：`is_conflict` 内容精化短路（Go `InConflictWith` 语义）。
4. **`ignore_delete` 文件夹选项**（Go `ignoreDelete` 对齐，puller 删除分支按配置跳过）。
5. **v3.1.0 准入线**：双端新二进制后启动 72h 耐久跑测。
6. **relay / global discovery 官方链路**与 Android 真机互联（需真实环境）。
7. 小项：pid 文件清理瑕疵；配置校验错误应落日志（目前仅弹窗）。

## 六、运维备忘

- WSL 构建 Linux 版：`RUSTC_WRAPPER= SCCACHE_DISABLE=1 CARGO_TARGET_DIR=$HOME/syncthing-rust-target cargo build --release -p syncthing`（WSL sccache 后端不通）。
- 无参数启动 = Auto（daemon + 托盘，需交互式会话；Session 0 无托盘图标）。单实例锁下二次启动自动转 TUI。
- Go 侧测试资产：`C:\Users\22414\AppData\Local\Microsoft\WinGet\...\syncthing.exe`（v2.1.1），互联配置在 `target/interop/`。

---

*关联文档：[`KNOWN_ISSUES.md`](../KNOWN_ISSUES.md) §20~§22、[`plans/CONFLICT_RESOLUTION_ALIGNMENT_2026-07-21.md`](../plans/CONFLICT_RESOLUTION_ALIGNMENT_2026-07-21.md)、[`plans/POST_V3_0_3_ROADMAP.md`](../plans/POST_V3_0_3_ROADMAP.md)*
