---
type: ADR
title: ADR-20261105-rest-api-auth：REST API 认证策略
description: API key 认证中间件的判定链——health/metrics 豁免、loopback 放行、全读写 key、只读 key、空 key 放行
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-rest-api-auth：REST API 认证策略

> English summary: The REST API authenticates every request through a
> single middleware with a fixed decision chain: `/rest/health` and
> `/metrics` are exempt; loopback clients are allowed without a key (local
> tooling/debugging); a valid full-access key allows any method; a valid
> read-only key allows `GET`/`HEAD`/`OPTIONS` only (everything else gets
> 403); if no key is configured (or configured keys are empty) all requests
> are allowed. Anything else is rejected with 401. Keys are accepted via the
> `x-api-key` header or the `X-API-Key` query parameter.

## 问题

REST API 默认监听 loopback（`DEFAULT_API_ADDR`），但需要同时满足三类
访问者：本地人工具（CLI/TUI/MCP bridge，无 key）、远程 GUI/脚本（持
全读写 key）、只读监控端点抓取（持只读 key）。认证中间件必须在不误伤
本地访问的前提下，对远端请求强制执行 key 校验，并区分读写权限。

## 备选方案

1. **一律要求 key**：最严格的方案，但本地 CLI/TUI 每次调用都要配置
   key，与「loopback 默认安全」的部署模型冲突（Go syncthing 同样对
   loopback 不做 GUI 认证）。
2. **按路径白名单**：只对敏感路径要求认证。维护成本高，新增端点容易
   漏配。
3. **判定链 + loopback 放行**（采纳）：单一中间件按固定顺序裁决，把
   「本地免 key」作为显式第一条规则。

## 决定

判定顺序（`api_key_middleware`，先到先裁决）：

1. `GET /rest/health` 与 `/metrics` 无条件放行（监控/探活不应依赖
   凭证；该结论与本 ADR 同时适用于 Prometheus 抓取与容器探针）。
2. 对端地址为 loopback 时无条件放行，支持本地调试与进程内组件调用。
3. `x-api-key` 头（或 `X-API-Key` 查询参数，大小写不敏感）等于
   全读写 key 时放行任意方法。
4. 等于只读 key 时仅放行 `GET`/`HEAD`/`OPTIONS`，其余方法返回
   `403 FORBIDDEN`（认证成功但越权）。
5. 若全读写 key 与只读 key 均未配置或均为空字符串，视为「未启用
   认证」，放行全部请求——保持与旧版本无 key 行为兼容。
6. 其余一律 `401 UNAUTHORIZED`。

## 理由

- loopback 放行把信任边界收在传输层（loopback 无法被远端伪造），避免
  为本地工具链分发凭证；这与威胁模型一致——API key 防御的是**远端**
  未授权访问与 CSRF 面，不防御本地进程。
- 只读 key 返回 403 而非 401：客户端已认证（key 有效），只是方法不
  被允许，便于调用方区分「换 key」与「换方法」。
- 空 key 放行是刻意的向后兼容：升级后未配置 key 的旧部署行为不变。

## 已知边界

- 反向代理场景下若代理把远端请求标为 loopback（如代理跑在本机），
  远端可绕过认证——部署文档须提醒「反向代理后应配置 API key」。
- `X-API-Key` 查询参数会进入访问日志，属便利性折衷，生产部署应优先
  使用请求头。
