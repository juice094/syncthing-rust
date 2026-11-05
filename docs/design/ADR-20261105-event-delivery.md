---
type: ADR
title: ADR-20261105-event-delivery：事件投递（WebSocket + REST long-poll）
description: EventBus 单一 publish 入口、双通道投递（WebSocket 直连 + REST 轮询缓冲）、连接生命周期
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-event-delivery：事件投递（WebSocket + REST long-poll）

> English summary: All events flow through a single `EventBus::publish`
> entry point. Each publish writes an `EventMessage` into a bounded ring
> buffer (for REST long-poll `since` catch-up), broadcasts over a
> `tokio::sync::broadcast` channel (for in-process subscribers), and fans
> out to registered WebSocket connections via per-connection `mpsc` senders
> (dropping on backpressure). WebSocket delivery applies an optional
> per-connection event-type filter; REST long-poll serves matching buffered
> events immediately and otherwise subscribes and waits up to 60 seconds.
> Sequence diagram: `docs/design/sequence/event-delivery.md`.

## 问题

Syncthing 生态有两类事件消费者：实时 GUI（WebSocket 长连接）与脚本/
旧客户端（Go 兼容的 `GET /rest/events?since=N` 长轮询）。事件总线必须
用一份 publish 入口同时喂饱两类消费者，且 REST 轮询在客户端断连期间
不能丢事件（`since` 语义要求可补拉）。

## 备选方案

1. **只实现 WebSocket，轮询靠客户端自己轮询状态接口**：违背 Go 兼容
   目标，`/rest/events` 是事实标准端点。
2. **两条独立管道**：REST 轮询与 WebSocket 各订阅各的源，publish 方需
   知悉所有管道。新增消费者类型时要改 publish 方。
3. **单一 publish + 广播通道 + 环形缓冲 + 连接表**（采纳）：publish
   只做三件事，消费者类型对发布方透明。

## 决定

- **EventBuffer**：容量 1000 的 `VecDeque` 环形缓冲，支持 `since(id)`
  与 `latest_id()`；满时挤掉最旧事件。REST 轮询的补拉窗口上限即此
  容量——`since` 距今超过 1000 个事件时只能拿到窗口内部分。
- **publish 三步**（`EventBus::publish`，非 async）：
  1. 转换为 `EventMessage`（单调递增全局 ID + 事件类型 + Unix 毫秒
     时间戳 + 去掉 `type` 字段的 JSON 负载——类型由信封字段承载）并
     推入 `EventBuffer`；
  2. `broadcast::send` 投递给所有进程内订阅者（无活跃接收者时仅
     warn，不视为错误）；
  3. `tokio::spawn` 异步任务遍历 WebSocket 连接表，对每个连接
     `try_send`；连接通道满（客户端慢）时丢弃该事件而不是阻塞
     publish 路径。
- **WebSocket 生命周期**：`websocket_handler` 升级后注册连接（`mpsc`
  容量 100）、spawn 转发任务按可选事件类型过滤后发送 JSON 文本帧；
  客户端关闭或出错时注销连接并 abort 转发任务。客户端 `ping` /
  `subscribe` / `unsubscribe` 文本消息目前只记日志（订阅变更尚未实现，
  连接级过滤在握手查询参数上）。
- **REST long-poll**（`events_poll_handler`，Go `GET /rest/events` 兼容，
  `limit` 钳制 1–1000）：先查缓冲，有匹配事件立即返回；否则订阅广播
  通道，以 1 秒粒度 `timeout` 循环最多 60 秒，收够 `limit` 或超时即返回
  已收集部分。`since` 过滤用事件 ID 单调性（`msg.id <= since` 跳过）。

## 理由

- 环形缓冲 + 广播通道覆盖了「断连补拉」与「实时」两种消费模式，且
  publish 保持 O(连接数) 的非阻塞开销。
- WebSocket 背压选择丢弃而非阻塞：事件流是尽力送达语义（Go 版同样
  不保证慢客户端不丢事件），不能拖垮发布方。
- 1 秒粒度的长轮询循环让「等到事件」与「60 秒总超时」两个条件都能
  及时生效，避免单个 `timeout(60s)` 收不到事件时无法提前因 limit
  满足而返回。

## 已知边界

- `EventBuffer` 用 `std::sync::Mutex`：锁内无 `.await`，且 publish 为
  非 async 函数，不会持有锁跨 await 点。
- WebSocket 客户端的订阅变更消息（`subscribe`/`unsubscribe`）尚未生效，
  只记日志——过滤参数以连接建立时的查询字符串为准。
