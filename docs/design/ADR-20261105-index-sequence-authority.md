---
type: ADR
title: ADR-20261105-08: 序列号单一权威源（数据库）
description: IndexManager 与 Scanner 共享数据库序列号单一权威源的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-08: 序列号单一权威源（数据库）

> English summary: The database — not any in-memory manager — is the single
> source of truth for per-folder sequence numbers; `IndexManager` never
> caches its own counter, so scanner- and index-assigned sequences stay
> contiguous across restarts.

## 问题

同一个 folder 的序列号由两条路径分配：扫描变更（`Scanner`，经
`LocalDatabase::increment_sequence`）与索引更新（`IndexManager::update_index`
/ `update_index_delta`）。如果 `IndexManager` 在内存里维护自己的计数器，
两条路径就会产生两个发散的序列空间：重启后恢复的值可能与数据库实际
最大值不一致，增量索引（`get_index_delta`）会漏发或重发文件。

## 备选方案

1. **IndexManager 内存自增，启动时从数据库恢复**：看似省一次 I/O，但
   恢复窗口（`new()` 返回后、第一次 `update_index` 前）Scanner 可能已
   分配了新序列号，内存值立即过期——正是双路径分歧的温床。
2. **数据库为唯一权威源**：每次分配都走 `increment_sequence`，内存里的
   `local_max_sequence` 只是缓存视图，每轮更新后从数据库重新读取。

## 决定

采用方案 2：

- `IndexManager::new` 用 `db.get_sequence()` 恢复 `local_max_sequence`，
  用 `get_folder_index_meta` 恢复 IndexID；IndexID 持久化于
  `update_folder_index_meta`。
- `update_index` / `update_index_delta` 内逐文件调用
  `db.increment_sequence()`，最后再以 `db.get_sequence()` 刷新缓存视图。
- **IndexID 的获取必须先于序列号分配**：新 folder 首次更新时，先确定
  （或生成）IndexID 再分配序列号，避免"新 IndexID 隐含序列号清零"把
  已分配的序列号作废。
- `update_index_id`（重大变更/重建索引）同时重置数据库序列号并持久化
  新 IndexID，保证二者永远一致。

## 理由

单一权威源消灭了双路径分叉的可能：Scanner 与 IndexManager 看到的是
同一个单调序列空间，重启后 `IndexManager::new` 恢复的值与数据库一致，
增量索引按 `(IndexID, remote_max_sequence)` 推导不会漏发。代价是每次
分配一次数据库往返；在本项目规模下该开销可忽略，正确性优先。
