---
type: ADR
title: Bandwidth Throttle
description: 字节级带宽限速的配置契约、令牌桶实现与 REST 端点，收编既有 throttle_percent。
resource: ./ADR-20261106-bandwidth-throttle.md
tags: [adr, sync, network, throttle, rest]
status: partially-implemented
project: syncthing-rust
timestamp: 2026-11-06T00:00:00Z
---

# ADR: Bandwidth Throttle

> **Implementation status (2026-11-06)**: Receive-side throttling is wired
> end-to-end (`Puller` -> `FolderModel` -> `SyncService::add_folder`), driven by
> `Options.max_recv_kbps` at folder creation time. **Not yet done**: the
> `POST /rest/system/throttle` endpoint (runtime hot-update), per-device
> override (`Device.max_recv_kbps`), and send-side throttling in `block_server`.
> Sections below describe the full target design; treat unimplemented parts as
> planned, not shipped.

## Context

当前 orchestrator.rs 的 set_throttle(percent) 注释明确写着：Currently acts as a control signal for the health predictor; the concurrency limits remain configuration-driven. 它不限制任何字节速率。AGENTS.md 的硬红线要求新依赖能不加就不加，所以不引入 governor。

同时，REST 路由表里没有任何 throttle 相关端点，这个旋钮目前没有外部入口，是既成事实的孤岛。

## Decision

### 配置契约

加在 Config.options 里（syncthing-core/src/types/mod.rs 的 Options 结构）：

    pub struct Options {
        // 现有字段保持不变
        /// 接收侧全局带宽上限，单位 KiB/s。None 表示不限。
        pub max_recv_kbps: Option<u64>,
        /// 发送侧全局带宽上限，单位 KiB/s。None 表示不限。
        pub max_send_kbps: Option<u64>,
    }

单位选 KiB/s 而非 Kbps，避免与网络运营商口径混淆。

### 实现位置

手写令牌桶，放在 syncthing-sync/src/throttle.rs（不新建 crate）：

    pub struct TokenBucket {
        rate_bytes_per_sec: u64,
        capacity: u64,
        tokens: AtomicU64,
        last_refill: Mutex<Instant>,
    }

    impl TokenBucket {
        pub async fn acquire(&self, n: u64) {
            // 不足则 sleep 到补足为止
        }
    }

- 接收侧：puller::download_file 在 file.write_all 前 acquire(block_data.len())。
- 发送侧：block_server 在返回块数据前 acquire(data.len())。
- 每设备独立桶：从 Config.devices[i] 的 max_recv_kbps 读取，缺省回退到全局 Options.max_recv_kbps。

### 与旧 throttle_percent 的关系

保留 throttle_percent（健康预测器的并发节流信号），但在 REST 层把两者合并为一个端点，避免用户困惑：

POST /rest/system/throttle

请求体字段：percent、max_recv_kbps、max_send_kbps。

percent 写 orchestrator.set_throttle()，max_*_kbps 写 Config.options 并重建令牌桶。三者互不覆盖，各自独立生效。

### 配置可见性

GET /rest/config 与 PUT /rest/config 已存在，Options 新字段自动随 Config 序列化可见。POST /rest/system/throttle 是运行时热更新入口，不需重启。

## Consequences

- 正面：字节级硬限速；旧 throttle_percent 不再是孤岛，有了 REST 入口；配置与热更新分离，语义清晰。
- 负面：每块 acquire 增加一次原子操作与潜在 sleep；高并发下锁竞争。
- 风险：令牌桶 capacity 设为 rate 的两倍，避免小包饥饿。需压测确认 acquire 不成为瓶颈。

## Alternatives Considered

- 引入 governor crate：成熟但违反能不加就不加。
- 只做并发限流不做字节限速：无法阻止单连接吃满带宽，不满足生产场景。
- 把 throttle_percent 直接改造成字节限速：语义突变，破坏健康预测器依赖。
