---
type: Design Note
title: Agent Integration Surfaces (Skill / MCP / DSH)
description: syncthing-rust 对外暴露给 AI Agent 生态的三条兼容面：仓内 Skill、MCP Bridge、以及对 DeepSeek Harness (DSH) 插件的兼容性评估结论。
resource: ./AGENT_INTEGRATION.md
tags: [agent, integration, mcp, skill, dsh, design]
status: active
project: syncthing-rust
timestamp: 2026-10-16T00:00:00Z
---

# Agent 集成面（Skill / MCP / DSH）

本文汇总 syncthing-rust 面向 AI Agent 生态的三条兼容面，以及 DSH 插件方向的评估结论。
**本仓不引入任何运行时依赖**；三条面都只是"把已有能力暴露出去"。

---

## 1. 三条兼容线总览

| 线 | 载体 | 位置 | 本轮状态 |
|:--|:--|:--|:--|
| Skill | `.agents/skills/bep-protocol/SKILL.md` | 本仓（仓库内技能，Trae / Claude 等可直接加载） | 已按 v3.0.4 事实重写 |
| MCP | `cmd/syncthing-mcp-bridge`（stdio JSON-RPC → REST） | 本仓 | 工具 11 → 19、新增 4 项集成测试、支持 `SYNCTHING_API_KEY` |
| DSH 插件 | DeepSeek Harness 桌面端插件（cordis 体系） | **Gray-Harness 仓**（`packages/`） | 仅评估结论，未落地代码（见 §3） |

---

## 2. MCP Bridge

```
Agent (Trae / Claude / Kimi)
  └─ MCP stdio (JSON-RPC 2.0) ──▶ syncthing-mcp-bridge ──▶ HTTP REST ▶ syncthing daemon
```

- **入口**：`cmd/syncthing-mcp-bridge`（单文件 JSON-RPC 实现 + `src/tools.rs` 名册，零第三方 MCP SDK 依赖）。
- **环境变量**：`SYNCTHING_API_URL`（默认 `http://127.0.0.1:8385`）、`SYNCTHING_API_KEY`（可选，作为 `X-API-Key` 转发；loopback 访问由服务端放行，故默认无需设置）。
- **工具（19）**：

| 分组 | 工具 |
|:--|:--|
| 设备 | `list_devices`、`get_device`、`add_device`、`remove_device` |
| 文件夹 | `list_folders`、`get_folder_status`、`add_folder`、`remove_folder`、`browse_folder`、`get_file`、`pause_folder`、`resume_folder` |
| 状态 / 观测 | `get_system_status`、`get_connections`、`get_db_status`、`get_health`、`get_metrics`、`poll_events` |
| 操作 | `trigger_scan` |

- **资源（3）**：`syncthing://status`、`syncthing://devices`、`syncthing://folders`。
- **编码约定**：folder/file 等取值一律走 `encode_component()` 百分号编码后再拼 URL（避免空格 / `&` / `#` 破坏请求）。
- **集成测试**：`cmd/syncthing-mcp-bridge/tests/mcp_bridge.rs` —— 启动进程内 stub HTTP 服务端（手写最小 HTTP/1.1，不引入框架），断言 `tools/list` 名册、REST 落点、`X-API-Key` 转发、查询串编码。
- **对应 REST 契约**：端点清单见 [`docs/agent/claude_reference.md`](../agent/claude_reference.md)；鉴权与边界见 [`docs/agent/security.md`](../agent/security.md)（loopback 免鉴权，非 loopback 需 Key）。

---

## 3. DSH 插件兼容性评估（结论：可做，但落点在 Gray-Harness 仓）

**DSH** = DeepSeek Harness 桌面端（`C:\Users\22414\Gray-Harness\deepseek-harness`）基于 cordis 的插件体系，
其插件契约写在 Gray-Harness 的 `docs/plugin-development-plan.md`：**不碰 fork core**、界面一律走 `ctx.slots.register`、
host 产物 `lib/` 零裸外部依赖、新能力优先走官方 RPC（`ctx.remote`）而非自建 loopback 路由、双半边（`src/host` + `src/client`）产物入库。

### 3.1 可行性

- syncthing-rust 侧的所需能力**已全部就绪**，无需为其新增接口：状态/连接查询走 `/rest/system/status`、`/rest/system/connections`，
  索引用量走 `/rest/db/status`、`/rest/folder/:id/status`，操作走 `/rest/scan/:id`、`/rest/pause/:id`、`/rest/resume/:id`，
  事件走 `/rest/events/poll`（或 WebSocket `/rest/events`）。
- 与本仓的耦合面只有一个：**REST 契约**（路径 + JSON 形状）。因此 DSH 插件不会污染本仓的 crate 边界与依赖图。

### 3.2 落点与形态（推断，依据 plugin-development-plan.md）

1. 插件源码放 Gray-Harness 的 `packages/gray-syncthing/`，双半边：
   - `src/host/`：访问 `127.0.0.1:8385` REST，并通过官方 RPC 暴露"查询状态 / 触发扫描 / 暂停恢复"；
   - `src/client/`：只做 `ctx.slots.register` 的状态条 / 面板渲染，不自建 store、不抢路由。
2. **不能**让 client 半边直连 REST：渲染进程直连 `http://127.0.0.1:8385` 会受同源/CSP 约束，且违反"新能力优先走官方 RPC"的约束——必须由 host 半边代理。
3. 需要同步改 Gray-Harness 的三处声明（`tsconfig.json` include、README 名册、`packages/gray-statusline/src/host/index.ts` 的 `BUS_MAP_EXPECTATIONS`），
   并在 `~/.dsh/profiles/desktop/package.json` 加 `link:` + `dsh.profile.bundles`（**改动前先做时间戳备份**，该层出问题会导致桌面端起不来）。

### 3.3 风险

| 风险 | 说明 | 缓解 |
|:--|:--|:--|
| 跨仓维护 | 插件代码与门禁都在 Gray-Harness，本仓无法单侧验证 | 本仓只承诺 REST 契约稳定；契约变更需同步通知 |
| 挂载层高风险 | `~/.dsh` 改动失败会直接导致桌面端无法启动 | 改动前时间戳备份；改后 `pnpm roster` + `pnpm roster:live` |
| 权限面 | REST 写操作（scan/pause/resume）需要 admin key（非 loopback 时） | 插件默认只读；写操作显式开关 + 文档提示 |
| 双份事实 | Gray-Harness 已因"同一事实 7 处手写"付过代价 | 名册/端点清单只在本仓留一份（本文 §2） |

### 3.4 建议

- **本轮不落地代码**：DSH 插件的作业面属于 Gray-Harness 仓（量级参考其 G7/G8 卡），与本仓"下一轮迭代"不冲突但也不该混在同一次提交里。
- 落地时的顺序：先在本仓确认/冻结 REST 契约清单 → Gray-Harness 建 `gray-syncthing` 包（host 半边优先、只读能力先行）→ 进门禁与名册 → 挂载层改 `~/.dsh`（备份）→ 运行态验收。
- 若要降低耦合，可复用 §2 的 MCP 工具名册作为端点契约的单一定义来源：DSH host 半边与 MCP Bridge 调用同一组 REST 路径。
