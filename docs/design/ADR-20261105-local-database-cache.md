---
type: ADR
title: ADR-20261105-07: LocalDatabase 缓存、批量落盘与序列号锁
description: FileSystemDatabase 的内存缓存软上限、并发批量落盘与 per-folder 序列号锁设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-07: LocalDatabase 缓存、批量落盘与序列号锁

> English summary: `FileSystemDatabase` keeps an in-memory cache with a soft
> cap and whole-folder random eviction, flushes batches concurrently, and
> serializes per-folder sequence read-modify-write with a mutex to avoid
> interleaved writes.

## 问题

`LocalDatabase` 的实现 `FileSystemDatabase` 需要同时满足三个互相拉扯的
目标：

1. 高频路径（pull 后回写、索引处理、扫描）不能每次都做磁盘 I/O；
2. 批量更新（一次扫描产生 N 个变更）不能退化为 O(N) 串行 syscall；
3. 序列号是全局单调的共享状态，多任务并发读写必须不产生撕裂写。

## 备选方案

1. **纯磁盘读写（无缓存）**：正确性最好，但每次 `get_file` 都是一次
   I/O，扫描比对与 pull 状态检查会被拖慢一个数量级。
2. **无上限缓存**：内存随文件夹规模线性增长；百万级文件目录不可接受。
3. **LRU 逐文件驱逐**：实现复杂（需要维护链表 + 锁），而本场景驱逐
   本身极少触发（仅在超过软上限时），简单策略的收益/成本比更高。
4. **批量落盘串行 await**：N 个文件的 fsync 串行化，批量扫描回写延迟
   随文件数线性放大。

## 决定

- **内存缓存 + 软上限**（默认 100K 文件，`AtomicUsize` 计数）：超过上限
  时随机驱逐**整个 folder** 的缓存条目（`evict_one_folder`），而非逐文件
  LRU。驱逐粒度粗，但实现极简且驱逐路径极冷。
- **批量更新并发落盘**：`update_files` 用 `JoinSet` 并发执行逐文件
  `update_file`，把 O(N) 串行 syscall 压成并发；单文件失败即报错返回。
- **per-folder 序列号锁**：`seq_locks: DashMap<String, Arc<Mutex<()>>>`
  串行化 `get_sequence` / `increment_sequence` / `reset_sequence` 的
  读-改-写。没有这把锁时，两个并发 `increment_sequence` 会交错执行
  `tokio::fs::write`（truncate-then-write），产生空文件或重复序列号
  （"empty string" 解析错误的根因）。
- `MemoryDatabase::check_globals` 与 `FileSystemDatabase::check_globals`
  目前是简化实现：只返回本地已知版本，**未查询真正的全局数据库**。
  这是已登记的实现上限，升级路径是引入全局版本表。

## 理由

- 缓存软上限兜住内存，随机整目录驱逐把复杂度压到最低（ponytail：
  按 folder 粒度驱逐足够，逐文件 LRU 是过度设计）。
- 并发落盘把批量回写的延迟从 O(N) syscall 降为并发等待。
- 序列号锁是对"共享可变状态必须串行化"的最小正确解；按 folder 分键
  使不同文件夹之间互不阻塞。
