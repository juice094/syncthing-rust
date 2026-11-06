---
type: ADR
title: Block Resume Persistence
description: 块级续传进度的存储契约、清理策略与 REST 查询端点。
resource: ./ADR-20261106-block-resume-persistence.md
tags: [adr, sync, storage, resume, rest]
status: proposed
project: syncthing-rust
timestamp: 2026-11-06T00:00:00Z
---

# ADR: Block Resume Persistence

> **Implementation status (2026-11-06)**: Not implemented. G1 was confirmed by
> a second-pass code search (`completed_block` / `resume_block` / `block_state` /
> `pending_block` / `in_flight` all return zero persistence hits; the only
> `in_flight` is an in-memory `pull_in_flight` flag in `folder_model`). This ADR
> is the agreed design, pending implementation.

## Context

puller::download_file 以 Semaphore 控制并发块请求，但接收进度只存在于内存。网络中断或进程重启后，未完成文件需要整文件重拉。MAX_BEP_MESSAGE_SIZE 为 128 MiB，大文件重拉成本是分钟级，72h 耐久测试会因此出现长尾不稳定。

## Decision

### 存储契约

新增 trait BlockProgressStore，定义在 syncthing-core/src/traits/mod.rs（只放 trait/类型/常量，符合 crate 边界红线）。

    #[async_trait]
    pub trait BlockProgressStore: Send + Sync {
        async fn record_block(&self, folder: &FolderId, file: &str, block_index: u32, hash: BlockHash) -> Result<()>;
        async fn get_progress(&self, folder: &FolderId, file: &str) -> Result<Vec<(u32, BlockHash)>>;
        async fn clear_progress(&self, folder: &FolderId, file: &str) -> Result<()>;
        async fn sweep_stale(&self, older_than: SystemTime) -> Result<usize>;
    }

### key 布局

progress/<folder_id>/<url_encoded_file_name>/<block_index_big_endian_u32> -> hash(32 bytes) + recorded_at_secs(8 bytes)

文件名做 URL 编码，防止斜杠和冒号破坏前缀扫描。block_index 用大端固定 4 字节，保证 sled 的字典序与索引序一致。

### 清理策略

- 文件成功 rename 落盘后立即 clear_progress。
- 每 10 分钟跑一次 sweep_stale，清理 recorded_at 超过 24 小时且对应文件已不在索引中的记录。
- 进程启动时跑一次 sweep_stale，清理上次崩溃残留。

### REST 端点

GET /rest/db/progress?folder=<id>&file=<name>

响应字段：folder、file、total_blocks、received_blocks、received_indices。当 total_blocks 超过 4096 时省略 received_indices，仅返回计数，避免响应体过大。

### 消费方

- puller：拉取前查 get_progress 跳过已记录块；每块校验通过后 record_block；文件完成后 clear_progress。
- syncthing-api：GET /rest/db/progress 查询。
- syncthing-db：实现 BlockProgressStore，走 run_blocking，不暴露 sled 特有 API。

## Consequences

- 正面：断点续传可跨进程重启；REST 可观测；72h 耐久测试不再长尾。
- 负面：DB 写放大（每块一次写）；需 TTL 清理防孤儿堆积。
- 风险：record_block 每块同步写会拖慢拉取。缓解：批量写，每 32 块或每 2 秒 flush 一次，崩溃时最多丢 32 块进度。

## Alternatives Considered

- 写入 FileInfo 的 sidecar 字段：污染元数据模型，且 FileInfo 会同步给 peer，进度是本地状态，不应外发。
- 用内存 DashMap 加定期快照：实现简单，但崩溃窗口大，不满足 72h 耐久测试的确定性要求。
