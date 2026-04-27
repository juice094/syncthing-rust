# 计划文件索引 · syncthing-rust

> **维护原则**：计划不是墓碑，是活文档。过时计划必须归档，避免误导决策。
> **最后审计**：2026-04-27，详见 [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md)

---

## 当前有效计划（2 份）

| 文件 | 状态 | 说明 |
|------|------|------|
| [`POST_V0_2_0_ROADMAP.md`](./POST_V0_2_0_ROADMAP.md) | ✅ 活跃 | **当前权威路线图**。制定日期 2026-04-26，审计修正后 2026-04-27。含优先级矩阵（P0~P5）、分阶段执行计划、决策记录（ADR）。 |
| [`PHASE3_PLAN.md`](./PHASE3_PLAN.md) | ⚠️ 保留+勘误 | BepSession 硬化计划。3.1~3.3 已完成，3.4（72h stress test）⏳ 未完成。**顶部有勘误**：3.3 节验证实际为格雷侧旧版 Rust，非 Go。 |

## 审计报告

| 文件 | 说明 |
|------|------|
| [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md) | 全面审计 6 份计划 + AGENTS.md + 代码实际状态。含虚假声明识别、定位重定义、文件清理行动清单。 |

## 已归档计划（4 份 → `docs/archive/plans/`）

| 文件 | 归档理由 |
|------|----------|
| [`docs/archive/plans/MVP_RECOVERY_PLAN.md`](../../archive/plans/MVP_RECOVERY_PLAN.md) | Phase 1~3 已完成，Phase 4 被后续计划覆盖。文档存在拼接错误。 |
| [`docs/archive/plans/PHASE4_PLAN.md`](../../archive/plans/PHASE4_PLAN.md) | Week 排期已过期；含虚假声明（连接循环"已完成"无 commit 支撑）；TUI/压测/打包计划由新路线图接管。 |
| [`docs/archive/plans/WAVE3_PLAN.md`](../../archive/plans/WAVE3_PLAN.md) | NET-REBIND / NET-DIALER / SYNC-SUPERVISOR 任务已全部实现。 |
| [`docs/archive/plans/improvement-plan.md`](../../archive/plans/improvement-plan.md) | Exit Criteria 过于理想化（零 unwrap / audit 零漏洞 / Go GUI 完全兼容），与单人维护约束脱节。 |

## 计划演进关系

```
MVP_RECOVERY (Phase 1~3) ──→ PHASE3 (3.1~3.3 完成, 3.4 ⏳)
                                    │
                                    ▼
                           PHASE4 (已过期, 已归档)
                                    │
                                    ▼
                  POST_V0_2_0_ROADMAP (当前权威, 2026-04-27 审计修正)
                                    │
                    ┌───────────────┼───────────────┐
                    ▼               ▼               ▼
               P0: 72h压测    P0: 跨版本互通    P1: API 闭环
               P3: audit债务   P2: .stignore     P3: PCP/NAT-PMP
```

## 跨文件跳转速查

- **当前该做什么？** → [`POST_V0_2_0_ROADMAP.md`](./POST_V0_2_0_ROADMAP.md)
- **为什么 cargo audit 不再是 P0？** → [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md) §三、P0 评估
- **PHASE3 的 Go 验证声明为什么可疑？** → [`PHASE3_PLAN.md`](./PHASE3_PLAN.md) 顶部勘误横幅
- **项目阶段性定位是什么？** → [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md) §三、定位重定义
- **历史计划为什么被归档？** → [`PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md) §一、逐份判定
