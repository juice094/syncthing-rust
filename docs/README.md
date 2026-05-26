---
title: Documentation Home
type: index
status: active
project: syncthing-rust
tags: [index, navigation, moc]
---

# Documentation

本目录包含 `syncthing-rust` 项目的设计文档、验证报告、计划、运维指南与历史归档。

> ✅ **当前阶段（2026-05-14 post-v0.2.5）**：v0.2.5 已发布，端到端 sync 验证通过；正在进行 v0.2.6 运行时安全 hotfix。  
> **快速入口**: [plans/NEXT_STEPS_2026-05-14.md](plans/NEXT_STEPS_2026-05-14.md) — 当前活跃路线图（v0.2.6 hotfix H-1~H-6）  
> **历史缺陷追溯**: [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) §2 含 T2.6 RCA；§7 含运行时安全审查（INC-20260514-001）

---

## 📋 必读

| 文档 | 用途 |
|------|------|
| [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) | 已知缺陷登记（含 T2.6 RCA、ClusterConfig race 等） |

---

## 目录结构

```
docs/
├── README.md                          # 本文档（导航页）
├── KNOWN_ISSUES.md                    # 已知缺陷登记（必读）
├── design/                            # 活跃的设计文档
├── plans/                             # 计划与路线图
├── reports/                           # 验证报告与实现总结
├── operations/                        # 运维与部署指南
└── archive/                           # 历史归档（工作日志、早期报告）
```

---

## design/ — 设计文档

| 文档 | 内容 | 状态 |
|------|------|------|
| [NETWORK_DISCOVERY_DESIGN.md](design/NETWORK_DISCOVERY_DESIGN.md) | 自建网络发现层：Local Discovery + Global Discovery + STUN + UPnP + Relay 完整设计 | 🟢 活跃 |
| [TUI_DESIGN.md](design/TUI_DESIGN.md) | TUI 架构、交互流程、弹窗与快捷键设计 | 🟢 活跃 |
| [FEATURE_COMPARISON.md](design/FEATURE_COMPARISON.md) | Rust 实现与官方 Go Syncthing 的功能对标 | 🟢 活跃 |

---

## plans/ — 计划与路线图

> ⚠️ 本节为简表。完整且最新的计划索引见 [`plans/INDEX.md`](plans/INDEX.md)。

| 文档 | 内容 | 状态 |
|------|------|------|
| [NEXT_STEPS_2026-05-14.md](plans/NEXT_STEPS_2026-05-14.md) | v0.2.6 hotfix 行动计划（H-1~H-6，运行时安全）| ✅ 最新 |
| [NEXT_STEPS_2026-05-13.md](plans/NEXT_STEPS_2026-05-13.md) | v0.2.4→v0.2.5 发布周期归档 | 🗃️ 归档 |
| [POST_V0_2_0_ROADMAP.md](plans/POST_V0_2_0_ROADMAP.md) | 战略级路线图（P0~P5 矩阵 + ADR）| 🟢 活跃 |
| [TUNING_PLAN_2026-05-11.md](plans/TUNING_PLAN_2026-05-11.md) | 横向调优计划（T-A~T-G） | 🟢 活跃 |
| [PHASE4_PLAN.md](plans/PHASE4_PLAN.md) | Phase 4 目标（已被 NEXT_STEPS 覆盖）| 🗃️ 归档 |
| [PHASE3_PLAN.md](plans/PHASE3_PLAN.md) | Phase 3 目标：Push/Pull E2E、BEP 协议兼容 | ✅ 已完成 |
| [WAVE3_PLAN.md](plans/WAVE3_PLAN.md) | Wave 3 详细任务分解 | ✅ 已完成 |
| [improvement-plan.md](plans/improvement-plan.md) | 通用改进事项清单 | 🗃️ 已归档 |
| [MVP_RECOVERY_PLAN.md](plans/MVP_RECOVERY_PLAN.md) | 早期项目恢复计划 | 🗃️ 归档 |

---

## reports/ — 验证报告与总结

| 文档 | 内容 | 日期 |
|------|------|------|
| [STRESS_TEST_REPORT_2026-05-13.md](reports/STRESS_TEST_REPORT_2026-05-13.md) | 9h11m 压测完整分析（T-F1 死锁修复验证） | 2026-05-13 |
| [SESSION_SUMMARY_2026-05-12.md](reports/SESSION_SUMMARY_2026-05-12.md) | v0.2.3→v0.2.4 周期工程归档 | 2026-05-12 |
| [STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md](reports/STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md) | T-F1 死锁根因分析（RCA）| 2026-05-12 |
| [BASELINE_2026-05-12.md](reports/BASELINE_2026-05-12.md) | Scanner/Puller/BEP 性能基线 | 2026-05-12 |
| [LOCK_AWAIT_AUDIT_2026-05-12.md](reports/LOCK_AWAIT_AUDIT_2026-05-12.md) | 全工程跨 await 持锁审计 | 2026-05-12 |
| [UNWRAP_AUDIT_2026-05-12.md](reports/UNWRAP_AUDIT_2026-05-12.md) | unwrap/expect 审计 | 2026-05-12 |
| [IMPLEMENTATION_SUMMARY.md](reports/IMPLEMENTATION_SUMMARY.md) | 架构总览、crate 职责、当前实现状态 | 持续更新 |
| [VERIFICATION_REPORT_BEP_2026-04-11.md](reports/VERIFICATION_REPORT_BEP_2026-04-11.md) | 首次跨网络 BEP 互操作测试（Tailscale） | 2026-04-11 |
| [INTEROP_TEST_REPORT.md](reports/INTEROP_TEST_REPORT.md) | 本地互操作测试笔记 | 2026-04-11 |
| ~~PROJECT_STATUS.md~~ | ~~滚动项目状态快照~~ | ~~已移除，状态见 README.md 和 RELEASES.md~~ |

---

## operations/ — 运维与部署指南

| 文档 | 内容 | 适用场景 |
|------|------|----------|
| [TAILSCALE_GUIDE.md](operations/TAILSCALE_GUIDE.md) | 与 Tailscale 协同部署，零配置 NAT 穿透 | 跨家庭/4G/CGNAT 同步 |
| [PROXY_GUIDE.md](operations/PROXY_GUIDE.md) | 通过 SOCKS5/HTTP 代理（如 Watt Toolkit、clash）转发出站连接 | 加速 discovery/relay、合规审计 |

---

## archive/ — 历史归档

> 以下文档记录了项目早期的开发过程，保留用于追溯，**不作为当前决策依据**。

| 文档 | 类型 | 日期 |
|------|------|------|
| `TODAY_WORK_REPORT_*.md` (×3) | 工作日报 | 2026-04-09 / 04-15 / 04-17 |
| `STAGE_REPORT_SYNCTHING_2026-04-10.md` | 阶段报告 | 2026-04-10 |
| `ENGINEERING_ANALYSIS_2026-04-09.md` | 工程分析 | 2026-04-09 |
| `WAVE2_MILESTONE_REPORT.md` | Wave 2 里程碑回顾 | 2026-04 |
| `WAVE3_MILESTONE_REPORT.md` | Wave 3 里程碑回顾 | 2026-04 |
| `GITHUB_OPTIMIZATION.md` | GitHub 仓库优化笔记 | 2026-04 |

---

## 阅读建议

- **新协作者**: 先看 [`reports/IMPLEMENTATION_SUMMARY.md`](reports/IMPLEMENTATION_SUMMARY.md) 了解架构，再看根目录 `README.md` 编译运行。
- **架构决策**: [`design/ARCHITECTURE_DECISIONS.md`](design/ARCHITECTURE_DECISIONS.md) 是所有粗粒度架构决策的统一入口。
- **当前开发重点**: [`plans/NEXT_STEPS_2026-05-14.md`](plans/NEXT_STEPS_2026-05-14.md) 是 v0.2.6 hotfix 路线图，含 H-1~H-6 运行时安全修复。
- **部署使用**: 跨网络同步看 [`operations/TAILSCALE_GUIDE.md`](operations/TAILSCALE_GUIDE.md)；代理转发看 [`operations/PROXY_GUIDE.md`](operations/PROXY_GUIDE.md)。
- **稳定性证据**: [`reports/STRESS_TEST_REPORT_2026-05-13.md`](reports/STRESS_TEST_REPORT_2026-05-13.md) 含 9h+ 压测分析。
- **历史追溯**: 需要了解某个决策的背景时，查阅 `archive/` 中的工作日报。
