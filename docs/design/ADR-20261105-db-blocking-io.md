---
type: ADR
title: ADR-20261105-13: sled 同步 I/O 统一走阻塞线程池 + 超时
description: syncthing-db 所有 sled 调用经 run_blocking 派发到 tokio blocking 线程池并加 10s 超时的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-13: sled 同步 I/O 统一走阻塞线程池 + 超时

> English summary: sled has no async API, so every sled call in
> `syncthing-db` is dispatched through `blocking::run_blocking`, which runs
> the closure on tokio's blocking thread pool and wraps it in a 10-second
> timeout; a panic in the closure is mapped to an internal error instead of
> aborting the caller.

## 问题

`sled` 是纯同步嵌入式数据库：每次 `get` / `insert` / `flush` 都是可能
阻塞数毫秒到数秒的磁盘 I/O（含 fsync 与锁竞争）。`syncthing-db` 的公共
API 全部是 async，如果直接在 tokio 工作线程上调用 sled，一次慢磁盘
访问就会卡住一个 runtime 工作线程；并发够多时事件循环整体停转，上层
BEP 会话的收发超时随之雪崩。

## 备选方案

1. **在 async 函数内直接调用 sled**：零成本，但占用 runtime 工作线程，
   磁盘抖动直接转化为全局延迟尖峰。否决。
2. **`tokio::task::block_in_place`**：只在 multi-thread runtime 下可用，
   current_thread 上下文直接 panic；且与 `spawn_blocking` 相比没有
   超时能力。否决。
3. **自建专用线程池**：隔离最彻底，但引入线程生命周期管理、退出同步
   等额外复杂度，而 tokio blocking 池已提供等价物。收益/成本比不够。
4. **`spawn_blocking` + 超时（采用）**：把闭包移到 tokio 的阻塞线程池，
   再加 `tokio::time::timeout(10s)` 兜底。

## 决定

- 唯一的入口是 `blocking::run_blocking(name, f)`：内部
  `timeout(BLOCKING_TIMEOUT, spawn_blocking(f))`，
  `BLOCKING_TIMEOUT = 10s`。
- 超时映射为 `SyncthingError::timeout`；`spawn_blocking` 返回的
  `JoinError`（闭包 panic）映射为 `SyncthingError::Internal`，
  不让 panic 穿透到调用方。
- sled 的 `Db` / `Tree` 句柄内部是 `Arc`，闭包参数一律用 clone 后的
  句柄满足 `FnOnce + Send + 'static`，克隆本身零 I/O。
- 交互时序见 `docs/design/sequence/db-blocking-io.md`。

## 理由

方案 4 用标准库已有的设施（tokio blocking 池 + timeout）拿到方案 3 的
大部分隔离收益，代码量最小（ponytail：能一行调 `spawn_blocking` 就不
自建池）。10s 超时给"磁盘永久卡死"一个明确的失败边界——同步语义下
调用方拿到的仍是 `Result`，上层可以记录并走退避重试，而不是挂着等。

> ponytail: 超时固定 10s 是魔法值但足够保守（sled 正常操作毫秒级）；
> 升级路径是把超时做成 `run_blocking` 的参数并按调用点区分。
