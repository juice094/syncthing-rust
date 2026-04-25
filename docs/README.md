# Documentation

本目录包含 `syncthing-rust` 项目的设计文档、验证报告、计划与历史归档。

> **快速入口**: [design/NETWORK_DISCOVERY_DESIGN.md](design/NETWORK_DISCOVERY_DESIGN.md) — 当前活跃的设计文档（自建网络发现层）。

---

## 目录结构

```
docs/
├── README.md                          # 本文档（导航页）
├── design/                            # 活跃的设计文档
├── plans/                             # 计划与路线图
├── reports/                           # 验证报告与实现总结
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

| 文档 | 内容 | 状态 |
|------|------|------|
| [PHASE4_PLAN.md](plans/PHASE4_PLAN.md) | Phase 4 目标：TUI 增强、长时压测、生产打包 | 🔵 进行中 |
| [PHASE3_PLAN.md](plans/PHASE3_PLAN.md) | Phase 3 目标：Push/Pull E2E、BEP 协议兼容 | ✅ 已完成 |
| [WAVE3_PLAN.md](plans/WAVE3_PLAN.md) | Wave 3 详细任务分解 | ✅ 已完成 |
| [improvement-plan.md](plans/improvement-plan.md) | 通用改进事项清单 | 🔵 持续更新 |
| [MVP_RECOVERY_PLAN.md](plans/MVP_RECOVERY_PLAN.md) | 早期项目恢复计划 | 🗃️ 归档 |

---

## reports/ — 验证报告与总结

| 文档 | 内容 | 日期 |
|------|------|------|
| [IMPLEMENTATION_SUMMARY.md](reports/IMPLEMENTATION_SUMMARY.md) | 架构总览、crate 职责、当前实现状态 | 持续更新 |
| [VERIFICATION_REPORT_BEP_2026-04-11.md](reports/VERIFICATION_REPORT_BEP_2026-04-11.md) | 首次跨网络 BEP 互操作测试（Tailscale） | 2026-04-11 |
| [INTEROP_TEST_REPORT.md](reports/INTEROP_TEST_REPORT.md) | 本地互操作测试笔记 | 2026-04-11 |
| [PROJECT_STATUS.md](reports/PROJECT_STATUS.md) | 滚动项目状态快照 | 🗃️ 可能过时 |

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
- **当前开发重点**: [`design/NETWORK_DISCOVERY_DESIGN.md`](design/NETWORK_DISCOVERY_DESIGN.md) 是网络层下一阶段（Phase 5）的权威参考。
- **历史追溯**: 需要了解某个决策的背景时，查阅 `archive/` 中的工作日报。
