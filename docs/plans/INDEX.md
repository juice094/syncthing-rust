---
type: Index
title: Plans Index
description: syncthing-rust 计划与路线图的入口索引，汇总当前有效计划、已归档计划与审计报告。
resource: ./INDEX.md
tags: [plan, roadmap, index, okf]
status: active
project: syncthing-rust
timestamp: 2026-07-20T00:00:00Z
---

# 计划文件索引 · syncthing-rust

> **维护原则**：计划不是墓碑，是活文档。过时计划必须归档，避免误导决策。
> **最后审计**：2026-04-27，详见 [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md)
> **最后更新**：2026-07-20（v3.0.4 发布后整理：7 份已完成/被取代的文件移入 `docs/archive/plans/`，本目录仅保留 3 份有效计划 + 1 份审计报告）

---

## 当前有效计划（3 份）

| 文件 | 状态 | 说明 |
|------|------|------|
| [`POST_V3_0_3_ROADMAP.md`](./POST_V3_0_3_ROADMAP.md) | ✅ **最新后续计划** | v3.0.3 发布后制定、v3.0.4 发布后更新。覆盖 72h stress test（v3.1.0 准入线）、Symlink 同步、高延迟网络优化、Web GUI/QUIC 冻结说明、P0~P2 优先级矩阵与 ADR。 |
| [`TUNING_PLAN_2026-05-11.md`](./TUNING_PLAN_2026-05-11.md) | ✅ 活跃 | 横向调优。T-A~T-G 七大任务组。v0.2.4 周期完成 T-F1/F2/A1/B1/E1/G2，待执行 T-C/D3/F3。 |
| [`PHASE3_PLAN.md`](./PHASE3_PLAN.md) | ⚠️ 保留+勘误 | BepSession 硬化计划。3.1~3.3 已完成，3.4（72h stress test）⏳ 已调整为双节点真实网络 72h（v3.1.0 准入线）。 |

## 审计报告

| 文件 | 说明 |
|------|------|
| [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md) | 全面审计 6 份计划 + AGENTS.md + 代码实际状态。含虚假声明识别、定位重定义、文件清理行动清单。 |

## 已归档计划（→ `docs/archive/plans/`）

### NEXT_STEPS 系列

- [`NEXT_STEPS_2026-05-17.md`](../archive/plans/NEXT_STEPS_2026-05-17.md) — v0.2.9-rc1 维护轮次归档，由 POST_V3_0_3_ROADMAP.md 接管
- [`NEXT_STEPS_2026-05-15.md`](../archive/plans/NEXT_STEPS_2026-05-15.md) — v0.2.6 发布后行动计划，由 NEXT_STEPS_2026-05-17.md / POST_V3_0_3_ROADMAP.md 接管
- `NEXT_STEPS_2026-05-14.md` — v0.2.6 hotfix 完成归档（文件未保留），由 NEXT_STEPS_2026-05-15.md 接管
- `NEXT_STEPS_2026-05-13.md` — 已被 NEXT_STEPS_2026-05-14.md 取代（文件未保留）
- `NEXT_STEPS_2026-05-12.md` — 已被 NEXT_STEPS_2026-05-13.md 取代（文件未保留）
- `NEXT_STEPS_2026-05-11.md` — 已被 NEXT_STEPS_2026-05-12.md 取代（文件未保留）

### 路线图与阶段计划

| 文件 | 归档理由 |
|------|----------|
| [`POST_V0_2_0_ROADMAP.md`](../archive/plans/POST_V0_2_0_ROADMAP.md) | v0.2.0-beta 后历史路线图，由 POST_V3_0_3_ROADMAP.md 取代。 |
| [`MVP_RECOVERY_PLAN.md`](../archive/plans/MVP_RECOVERY_PLAN.md) | Phase 1~3 已完成，Phase 4 被后续计划覆盖。文档存在拼接错误。 |
| [`PHASE4_PLAN.md`](../archive/plans/PHASE4_PLAN.md) | Week 排期已过期；含虚假声明（连接循环"已完成"无 commit 支撑）；TUI/压测/打包计划由新路线图接管。 |
| [`WAVE3_PLAN.md`](../archive/plans/WAVE3_PLAN.md) | NET-REBIND / NET-DIALER / SYNC-SUPERVISOR 任务已全部实现。 |
| [`improvement-plan.md`](../archive/plans/improvement-plan.md) | Exit Criteria 过于理想化（零 unwrap / audit 零漏洞 / Go GUI 完全兼容），与单人维护约束脱节。 |

### 一次性分析 / 交接 / 运维快照（2026-07-20 归档）

| 文件 | 归档理由 |
|------|----------|
| [`CONFIG_SERIALIZATION_ANALYSIS_2026-04-27.md`](../archive/plans/CONFIG_SERIALIZATION_ANALYSIS_2026-04-27.md) | 配置序列化严格化分析，frontmatter 标记 completed，结论已落入代码。 |
| [`OFFLINE_HANDOVER_2026-04-27.md`](../archive/plans/OFFLINE_HANDOVER_2026-04-27.md) | 特定日期离线交接文档（含当时设备 ID/内网 IP），场景已过。 |
| [`SITUATION_2026-06-03.md`](../archive/plans/SITUATION_2026-06-03.md) | v0.2.10-rc2 阶段情况说明，frontmatter 标记 archived，内容为历史快照。 |
| [`GRAY_SIDE_OPS.md`](../archive/plans/GRAY_SIDE_OPS.md) | 针对旧测试环境（40GB VM）的互通验证指南，frontmatter 标记 archived，由 `docs/GRAY_CLOUD_OPS_MANUAL.md` 接管。 |

## 计划演进关系

```
MVP_RECOVERY (Phase 1~3) ──→ PHASE3 (3.1~3.3 完成, 3.4 ⏳ 72h 压测)
                                    │
                                    ▼
                           PHASE4 (已过期, 已归档)
                                    │
                                    ▼
                  POST_V0_2_0_ROADMAP (已归档)
                                    │
                                    ▼
                  POST_V3_0_3_ROADMAP (当前权威, v3.0.4 后更新)
                                    │
                    ┌───────────────┼───────────────┐
                    ▼               ▼               ▼
               P0: 72h压测    P1: 高延迟优化   P2: Symlink
               (v3.1.0 阻塞)  (v3.1.x~v3.2.0)  (v3.2.0)
```

## 跨文件跳转速查

- **当前该做什么？** → [`POST_V3_0_3_ROADMAP.md`](./POST_V3_0_3_ROADMAP.md)
- **战略级路线（P0~P2 矩阵 + ADR）？** → [`POST_V3_0_3_ROADMAP.md`](./POST_V3_0_3_ROADMAP.md)
- **如何调优 / 压测 / 拆大文件？** → [`TUNING_PLAN_2026-05-11.md`](./TUNING_PLAN_2026-05-11.md)
- **为什么 cargo audit 不再是 P0？** → [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md) §三、P0 评估
- **PHASE3 的 Go 验证声明为什么可疑？** → [`PHASE3_PLAN.md`](./PHASE3_PLAN.md) 顶部勘误横幅
- **项目阶段性定位是什么？** → [`../KNOWN_ISSUES.md`](../KNOWN_ISSUES.md) 顶部阶段定位表
- **历史计划为什么被归档？** → [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md) §一、逐份判定
- **安全审计 / 接受债务 / 漏洞报告？** → [`../../SECURITY.md`](../../SECURITY.md)
- **v0.2.4 周期单日归档？** → [`../reports/SESSION_SUMMARY_2026-05-12.md`](../reports/SESSION_SUMMARY_2026-05-12.md)
- **双节点部署任务书？** → [`../GRAY_CLOUD_OPS_MANUAL.md`](../GRAY_CLOUD_OPS_MANUAL.md)
