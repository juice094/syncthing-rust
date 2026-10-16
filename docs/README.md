---
type: Index
title: Documentation Home
description: syncthing-rust 文档目录的 OKF bundle 入口，索引设计文档、计划、报告、运维指南、事故复盘与历史归档。
resource: ./README.md
tags: [index, navigation, moc, okf]
status: active
project: syncthing-rust
timestamp: 2026-10-16T00:00:00Z
---

# Documentation

本目录包含 `syncthing-rust` 项目的设计文档、验证报告、计划、运维指南、事故复盘与历史归档。

> ✅ **当前阶段（v3.0.4）**：Production — 官方 Go v2.1.1 互联实测通过，Windows 托盘 + TUI 稳定。
> 测试基线与 CI job 组成见 [`agent/testing.md`](agent/testing.md)（**唯一事实源，勿在别处手抄**）。
> **快速入口**：[`plans/POST_V3_0_3_ROADMAP.md`](plans/POST_V3_0_3_ROADMAP.md) — 当前权威路线图（v3.0.4 后更新）。
> **历史缺陷追溯**：[`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) 是权威缺陷台账（§15 stale-index 误删 RCA、§7 运行时安全审查 INC-20260514-001）。

---

## 按角色进入

| 角色 | 推荐路径 |
|:---|:---|
| 🧑💻 **新协作者 / 开发者** | 根 [`CONTRIBUTING.md`](../CONTRIBUTING.md) → [`agent/index.md`](agent/index.md) → [`design/topology.md`](design/topology.md) → [`agent/constraints.md`](agent/constraints.md) |
| 📦 **使用者 / 部署者** | 根 [`README.md`](../README.md) → [`agent/operations.md`](agent/operations.md) → `operations/` 部署指南 |
| 🤖 **AI Agent** | 根 [`AGENTS.md`](../AGENTS.md) / [`CLAUDE.md`](../CLAUDE.md) → [`agent/index.md`](agent/index.md) → [`.agents/skills/bep-protocol/SKILL.md`](../.agents/skills/bep-protocol/SKILL.md) |

---

## 📋 必读

| 文档 | 用途 |
|:---|:---|
| [`agent/index.md`](agent/index.md) | **Agent 操作指引**：crate 边界、禁止事项、测试要求、安全、运维 |
| [`design/topology.md`](design/topology.md) | **项目拓扑与架构入口**：目录树、Crate DAG、运行时组件、关键入口 |
| [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md) | **权威缺陷台账**：含 T2.6 RCA、ClusterConfig race、stale-index 误删等 |

---

## 目录结构

```
docs/
├── README.md              # 本文档（唯一 OKF bundle 入口）
├── KNOWN_ISSUES.md        # 权威缺陷台账（必读）
├── agent/                 # Agent 操作指引（约束 / 测试 / 安全 / 运维 / Claude 参考）
├── design/                # 活跃设计文档（拓扑、架构决策、发现层、TUI、性能、Agent 集成）
├── plans/                 # 计划与路线图（INDEX.md 为权威索引）
├── reports/               # 验证报告、审计、压测、会话总结
├── operations/            # 运维、部署、代理、灾备、工程纪律
├── postmortems/           # 事故复盘
├── rfc/ · drafts/         # 设计提案与草稿
├── releases/ · handoffs/  # 版本说明、跨会话交接
└── archive/               # 历史归档（plans/ · verification-reports/ · obsidian-nav/ …）
```

---

## agent/ — Agent 操作指引

> 面向 AI 编程 Agent 的约束、测试、安全与运维 bundle。正文在此；仓根 [`AGENTS.md`](../AGENTS.md) / [`CLAUDE.md`](../CLAUDE.md) 只是入口页。

| 文档 | 内容 |
|:---|:---|
| [agent/index.md](agent/index.md) | Agent 指引入口与快速参考 |
| [agent/constraints.md](agent/constraints.md) | Crate 边界红线、禁止事项、代码风格、文件规模限制 |
| [agent/testing.md](agent/testing.md) | 测试基线（**唯一事实源**）、E2E / TestNode 要求、提交前清单 |
| [agent/security.md](agent/security.md) | 威胁模型、关键上限、审计债务、部署安全建议 |
| [agent/operations.md](agent/operations.md) | 构建产物、部署脚本、灾备恢复、CI/CD |
| [agent/claude_reference.md](agent/claude_reference.md) | Claude Code 专属：构建矩阵、feature flag、daemon 生命周期、CLI / TUI / REST 全表 |

---

## design/ — 设计文档

| 文档 | 内容 |
|:---|:---|
| [topology.md](design/topology.md) | **项目拓扑与架构总览**：目录树、Crate DAG、运行时组件、关键入口 |
| [ARCHITECTURE_DECISIONS.md](design/ARCHITECTURE_DECISIONS.md) | 粗粒度架构决策与冻结项记录（ADR 统一入口） |
| [NETWORK_DISCOVERY_DESIGN.md](design/NETWORK_DISCOVERY_DESIGN.md) | 网络发现层：Local + Global Discovery + STUN + UPnP + Relay 完整设计 |
| [TUI_DESIGN.md](design/TUI_DESIGN.md) | TUI 架构、交互流程、弹窗与快捷键设计 |
| [AGENT_INTEGRATION.md](design/AGENT_INTEGRATION.md) | MCP Bridge：19 工具 + 3 资源，Agent 集成方式 |
| [PROFILING.md](design/PROFILING.md) | 性能分析方法与工具 |
| [FEATURE_COMPARISON.md](design/FEATURE_COMPARISON.md) | Rust 实现与官方 Go Syncthing 的功能对标（2026-04-09 快照） |

---

## plans/ — 计划与路线图

> ⚠️ 权威且完整的计划索引见 [`plans/INDEX.md`](plans/INDEX.md)；已归档计划在 [`archive/plans/`](archive/plans/)。

| 文档 | 状态 |
|:---|:---|
| [POST_V3_0_3_ROADMAP.md](plans/POST_V3_0_3_ROADMAP.md) | ✅ 当前权威后续计划（v3.0.4 后更新） |
| [CONFLICT_RESOLUTION_ALIGNMENT_2026-07-21.md](plans/CONFLICT_RESOLUTION_ALIGNMENT_2026-07-21.md) | ✅ 待执行：冲突仲裁三阶段修复 |
| [TUNING_PLAN_2026-05-11.md](plans/TUNING_PLAN_2026-05-11.md) | 🟢 活跃：横向调优 T-A~T-G |
| [PHASE3_PLAN.md](plans/PHASE3_PLAN.md) | ⚠️ 保留 + 勘误：BepSession 硬化（3.4 72h 压测待办） |
| [PLAN_AUDIT_2026-04-27.md](plans/PLAN_AUDIT_2026-04-27.md) | 计划审计报告（含逐份判定与清理清单） |
| [INDEX.md](plans/INDEX.md) | 计划索引与跨文件跳转 |

---

## reports/ — 验证报告与总结

> 完整清单（22 份）见 `reports/` 目录；以下为代表性 / 权威报告。

| 文档 | 内容 | 日期 |
|:---|:---|:---|
| [SESSION_REPORT_2026-10-16.md](reports/SESSION_REPORT_2026-10-16.md) | CI 长期红转绿、三处产品缺陷、ClusterConfig 竞态根治 | 2026-10-16 |
| [SESSION_REPORT_2026-07-21.md](reports/SESSION_REPORT_2026-07-21.md) | 向量根因修复、数据事故加固、冲突 Go 对齐、e2e 归因 | 2026-07-21 |
| [CRUD_REPAIR_E2E_2026-05-22.md](reports/CRUD_REPAIR_E2E_2026-05-22.md) | E2E CRUD 5/5 修复验证报告 | 2026-05-22 |
| [DUAL_NODE_TEST_2026-05-15.md](reports/DUAL_NODE_TEST_2026-05-15.md) | Windows ↔ Ubuntu 真实网络双节点测试 | 2026-05-15 |
| [CODE_HEALTH_AUDIT_2026-05-15.md](reports/CODE_HEALTH_AUDIT_2026-05-15.md) | 代码健康度审计 | 2026-05-15 |
| [STRESS_TEST_REPORT_2026-05-13.md](reports/STRESS_TEST_REPORT_2026-05-13.md) | 9h11m 压测完整分析（T-F1 死锁修复验证） | 2026-05-13 |
| [STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md](reports/STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md) | T-F1 死锁根因分析（RCA） | 2026-05-12 |
| [IMPLEMENTATION_SUMMARY.md](reports/IMPLEMENTATION_SUMMARY.md) | 2026-04-26 之前的项目实现总结（已归档） | 2026-04-26 |

TUI 专项报告：[`reports/tui_review_and_optimization_plan.md`](reports/tui_review_and_optimization_plan.md) · [`reports/tui_optimization_roadmap_v2.md`](reports/tui_optimization_roadmap_v2.md)。

---

## operations/ — 运维与部署指南

| 文档 | 内容 |
|:---|:---|
| [GRAY_CLOUD_OPS_MANUAL.md](operations/GRAY_CLOUD_OPS_MANUAL.md) | Gray-Cloud Linux 节点部署、运维与真实网络双节点验证 SOP |
| [DISASTER_RECOVERY.md](operations/DISASTER_RECOVERY.md) | 灾备恢复 Runbook（RPO/RTO、备份策略、重建流程） |
| [TAILSCALE_GUIDE.md](operations/TAILSCALE_GUIDE.md) | 与 Tailscale 协同部署，零配置 NAT 穿透 |
| [PROXY_GUIDE.md](operations/PROXY_GUIDE.md) | 通过 SOCKS5/HTTP 代理转发出站连接 |
| [ENGINEERING_DISCIPLINE.md](operations/ENGINEERING_DISCIPLINE.md) | 工程纪律规范：部署测试、审查、文档同步、合并门禁 |

---

## postmortems/ — 事故复盘

| 文档 | 内容 |
|:---|:---|
| [2026-06-15-gray-cloud-tls-handshake-stall.md](postmortems/2026-06-15-gray-cloud-tls-handshake-stall.md) | Gray-Cloud TLS 握手停滞事件：daemon 活着但停止处理连接的复盘与时间线 |

---

## rfc/ · drafts/ · releases/ · handoffs/

| 文档 | 内容 |
|:---|:---|
| [rfc/TRANSPORT_PLUGIN_RFC.md](rfc/TRANSPORT_PLUGIN_RFC.md) | 传输插件 RFC（可插拔 Transport trait 提案） |
| [drafts/RFC-001-service-split.md](drafts/RFC-001-service-split.md) | SyncService 拆分草案（被 5 处源码注释引用） |
| [releases/v0.2.5.md](releases/v0.2.5.md) | v0.2.5 版本说明与校验和 |
| `handoffs/` | 跨会话工作交接（2026-05-16/17 ×3），已归档性质 |

---

## archive/ — 历史归档

> 早期开发过程材料，仅供追溯，**不作为当前决策依据**。

| 子目录 / 条目 | 内容 |
|:---|:---|
| `archive/plans/` | 已归档计划（NEXT_STEPS、PHASE4、POST_V0_2_0、MVP_RECOVERY 等） |
| `archive/verification-reports/` | 2025-04 早期验证报告 |
| `archive/2026-04-04-bep-handshake/` | BEP 握手阶段交付物（约 26 份） |
| `archive/deliverables-wave-1/` | Wave 1 子代理交付物 |
| `archive/obsidian-nav/` | 早期 Obsidian `[[wikilink]]` 导航层（`_Home` + 各目录 `_MOC`），2026-10-16 归档，**非当前入口** |
| 顶层 `TODAY_WORK_REPORT_*` · `WAVE*_MILESTONE_REPORT` · `*_ANALYSIS_*` | 工作日报、里程碑回顾、工程分析 |

---

## 阅读建议

- **新协作者**：先看 [`agent/index.md`](agent/index.md) 与 [`design/topology.md`](design/topology.md)，再按根 `README.md` 编译运行。
- **架构决策**：[`design/ARCHITECTURE_DECISIONS.md`](design/ARCHITECTURE_DECISIONS.md) 是所有粗粒度架构决策的统一入口。
- **当前开发重点**：[`plans/POST_V3_0_3_ROADMAP.md`](plans/POST_V3_0_3_ROADMAP.md) 是当前权威路线图。
- **部署使用**：跨网络同步看 [`operations/TAILSCALE_GUIDE.md`](operations/TAILSCALE_GUIDE.md)；代理转发看 [`operations/PROXY_GUIDE.md`](operations/PROXY_GUIDE.md)；Gray-Cloud 运维看 [`operations/GRAY_CLOUD_OPS_MANUAL.md`](operations/GRAY_CLOUD_OPS_MANUAL.md)；灾备看 [`operations/DISASTER_RECOVERY.md`](operations/DISASTER_RECOVERY.md)。
- **稳定性证据**：[`reports/CRUD_REPAIR_E2E_2026-05-22.md`](reports/CRUD_REPAIR_E2E_2026-05-22.md) 含 5/5 E2E CRUD 验证；[`reports/STRESS_TEST_REPORT_2026-05-13.md`](reports/STRESS_TEST_REPORT_2026-05-13.md) 含 9h+ 压测分析。
- **事故追溯**：[`postmortems/`](postmortems/) 与 [`KNOWN_ISSUES.md`](KNOWN_ISSUES.md)。
- **历史追溯**：需要了解某个决策的背景时，查阅 [`archive/`](archive/)。
