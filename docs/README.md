---
type: Index
title: Documentation Home
description: syncthing-rust 文档目录的 OKF bundle 入口，索引设计文档、计划、报告、运维指南与历史归档。
resource: ./README.md
tags: [index, navigation, moc, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# Documentation

本目录包含 `syncthing-rust` 项目的设计文档、验证报告、计划、运维指南与历史归档。

> ✅ **当前阶段（2026-07-20 v3.0.4）**：Production — 414 passed / 6 ignored / 0 failed，E2E 双向同步已实测，Windows 托盘 + TUI 稳定。  
> **快速入口**: [plans/POST_V3_0_3_ROADMAP.md](plans/POST_V3_0_3_ROADMAP.md) — 当前权威路线图（v3.0.4 后更新）  
> **历史缺陷追溯**: [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) §15 含 stale-index 误删 RCA；§7 含运行时安全审查（INC-20260514-001）

---

## 📋 必读

| 文档 | 用途 |
|------|------|
| [`agent/index.md`](agent/index.md) | **Agent 操作指引**：crate 边界、禁止事项、测试要求、安全、运维 |
| [`design/topology.md`](design/topology.md) | **项目拓扑与架构入口**：目录树、Crate DAG、运行时组件、关键入口、核心声明验证 |
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

## agent/ — Agent 操作指引

> 面向 AI 编程 Agent 的约束、测试、安全与运维 bundle。

| 文档 | 内容 | 状态 |
|------|------|------|
| [agent/index.md](agent/index.md) | Agent 指引入口与快速参考 | 🟢 活跃 |
| [agent/constraints.md](agent/constraints.md) | Crate 边界红线、禁止事项、代码风格、文件规模限制 | 🟢 活跃 |
| [agent/testing.md](agent/testing.md) | 测试基线、E2E / TestNode 要求、提交前检查清单 | 🟢 活跃 |
| [agent/security.md](agent/security.md) | 威胁模型、关键上限、审计债务、部署安全建议 | 🟢 活跃 |
| [agent/operations.md](agent/operations.md) | 构建产物、部署脚本、灾备恢复、CI/CD | 🟢 活跃 |

---

## design/ — 设计文档

| 文档 | 内容 | 状态 |
|------|------|------|
| [topology.md](design/topology.md) | **项目拓扑与架构总览**：目录树、Crate DAG、运行时组件、关键入口 | 🟢 活跃 |
| [ARCHITECTURE_DECISIONS.md](design/ARCHITECTURE_DECISIONS.md) | 粗粒度架构决策与冻结项记录 | 🟢 活跃 |
| [NETWORK_DISCOVERY_DESIGN.md](design/NETWORK_DISCOVERY_DESIGN.md) | 自建网络发现层：Local Discovery + Global Discovery + STUN + UPnP + Relay 完整设计 | 🟢 活跃 |
| [TUI_DESIGN.md](design/TUI_DESIGN.md) | TUI 架构、交互流程、弹窗与快捷键设计 | 🟢 活跃 |
| [PROFILING.md](design/PROFILING.md) | 性能分析方法与工具 | 🟢 活跃 |
| [FEATURE_COMPARISON.md](design/FEATURE_COMPARISON.md) | Rust 实现与官方 Go Syncthing 的功能对标（2026-04-09 快照） | 🗃️ 归档 |

---

## plans/ — 计划与路线图

> ⚠️ 本节为简表。完整且最新的计划索引见 [`plans/INDEX.md`](plans/INDEX.md)。

| 文档 | 内容 | 状态 |
|------|------|------|
| [POST_V3_0_3_ROADMAP.md](plans/POST_V3_0_3_ROADMAP.md) | v3.0.3 发布后后续计划（72h stress test、Symlink、网络优化）| ✅ 最新 |
| [TUNING_PLAN_2026-05-11.md](plans/TUNING_PLAN_2026-05-11.md) | 横向调优计划（T-A~T-G） | 🟢 活跃 |
| [POST_V0_2_0_ROADMAP.md](archive/plans/POST_V0_2_0_ROADMAP.md) | v0.2.0-beta 后历史路线图 | 🗃️ 归档 |
| [PHASE3_PLAN.md](plans/PHASE3_PLAN.md) | Phase 3 目标：Push/Pull E2E、BEP 协议兼容 | ✅ 已完成 |
| [PHASE4_PLAN.md](archive/plans/PHASE4_PLAN.md) | Phase 4 目标（已被后续路线图覆盖）| 🗃️ 归档 |
| [WAVE3_PLAN.md](archive/plans/WAVE3_PLAN.md) | Wave 3 详细任务分解 | ✅ 已完成 |
| [improvement-plan.md](archive/plans/improvement-plan.md) | 通用改进事项清单 | 🗃️ 已归档 |
| [MVP_RECOVERY_PLAN.md](archive/plans/MVP_RECOVERY_PLAN.md) | 早期项目恢复计划 | 🗃️ 归档 |

---

## reports/ — 验证报告与总结

| 文档 | 内容 | 日期 |
|------|------|------|
| [CRUD_REPAIR_E2E_2026-05-22.md](reports/CRUD_REPAIR_E2E_2026-05-22.md) | E2E CRUD 5/5 修复验证报告 | 2026-05-22 |
| [DUAL_NODE_TEST_2026-05-15.md](reports/DUAL_NODE_TEST_2026-05-15.md) | Windows ↔ Ubuntu 真实网络双节点测试 | 2026-05-15 |
| [CODE_HEALTH_AUDIT_2026-05-15.md](reports/CODE_HEALTH_AUDIT_2026-05-15.md) | 代码健康度审计 | 2026-05-15 |
| [STRESS_TEST_REPORT_2026-05-13.md](reports/STRESS_TEST_REPORT_2026-05-13.md) | 9h11m 压测完整分析（T-F1 死锁修复验证） | 2026-05-13 |
| [STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md](reports/STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md) | T-F1 死锁根因分析（RCA）| 2026-05-12 |
| [IMPLEMENTATION_SUMMARY.md](reports/IMPLEMENTATION_SUMMARY.md) | 2026-04-26 之前的项目实现总结（已归档） | 2026-04-26 |
| [REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md](reports/REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md) | 真实网络双节点 E2E 笔记 | 2026-05-18 |
| [WSL2_WINDOWS_DUAL_NODE_E2E_2026-05-18.md](reports/WSL2_WINDOWS_DUAL_NODE_E2E_2026-05-18.md) | WSL2 ↔ Windows 双节点 E2E | 2026-05-18 |
| ~~PROJECT_STATUS.md~~ | ~~滚动项目状态快照~~ | ~~已移除，状态见 README.md、RELEASES.md、KNOWN_ISSUES.md~~ |

---

## operations/ — 运维与部署指南

| 文档 | 内容 | 适用场景 |
|------|------|----------|
| [GRAY_CLOUD_OPS_MANUAL.md](GRAY_CLOUD_OPS_MANUAL.md) | Gray-Cloud Linux 节点运维手册 | 对侧 VPS 部署与 72h 压测 |
| [TAILSCALE_GUIDE.md](operations/TAILSCALE_GUIDE.md) | 与 Tailscale 协同部署，零配置 NAT 穿透 | 跨家庭/4G/CGNAT 同步 |
| [PROXY_GUIDE.md](operations/PROXY_GUIDE.md) | 通过 SOCKS5/HTTP 代理转发出站连接 | 加速 discovery/relay、合规审计 |

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

---

## 阅读建议

- **新协作者**: 先看 [`design/topology.md`](design/topology.md) 了解架构拓扑，再看根目录 `README.md` 编译运行。
- **架构决策**: [`design/ARCHITECTURE_DECISIONS.md`](design/ARCHITECTURE_DECISIONS.md) 是所有粗粒度架构决策的统一入口。
- **当前开发重点**: [`plans/POST_V3_0_3_ROADMAP.md`](plans/POST_V3_0_3_ROADMAP.md) 是当前权威路线图（v3.0.4 后更新）。
- **部署使用**: 跨网络同步看 [`operations/TAILSCALE_GUIDE.md`](operations/TAILSCALE_GUIDE.md)；代理转发看 [`operations/PROXY_GUIDE.md`](operations/PROXY_GUIDE.md)；Gray-Cloud 运维看 [`GRAY_CLOUD_OPS_MANUAL.md`](GRAY_CLOUD_OPS_MANUAL.md)。
- **稳定性证据**: [`reports/CRUD_REPAIR_E2E_2026-05-22.md`](reports/CRUD_REPAIR_E2E_2026-05-22.md) 含 5/5 E2E CRUD 验证；[`reports/STRESS_TEST_REPORT_2026-05-13.md`](reports/STRESS_TEST_REPORT_2026-05-13.md) 含 9h+ 压测分析。
- **历史追溯**: 需要了解某个决策的背景时，查阅 `archive/` 中的工作日报。
