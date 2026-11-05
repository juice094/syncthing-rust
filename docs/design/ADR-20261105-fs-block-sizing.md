---
type: ADR
title: ADR-20261105-17: 块大小分级与哈希并行度
description: scan_file 块大小 clamp 区间、optimal_block_size 四档分级、批量 32 块 rayon 池哈希、目录扫描有界并发的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-17: 块大小分级与哈希并行度

> English summary: `scan_file` clamps any requested block size into
> [4 KiB, 16 MiB]. `optimal_block_size` picks from four tiers by file
> size: 4 KiB below 1 MiB, 128 KiB below 100 MiB, 256 KiB below 1 GiB,
> 1 MiB above. Hashing reads the file in batches of 32 blocks (~4 MiB) and
> dispatches each batch to a dedicated rayon pool (isolated from tokio's
> blocking pool, which serves sled I/O). `scan_directory` scans entries
> with bounded concurrency (`num_cpus`, min 2) via `JoinSet`.

## 问题

1. **块粒度**：块是 BEP 索引与去重的最小单位。块太小 → 索引体积与
   逐块请求开销爆炸；块太大 → 增量同步的最小重传单元变大，小修改
   也要重传整块。且必须与 Go Syncthing 的默认（128 KiB）互通。
2. **哈希是 CPU 密集**：SHA-256 是纯计算，大文件扫描会把所有核吃满；
   若与 sled 的阻塞 I/O 共用 tokio blocking 池，磁盘任务会被哈希
   饿死，反之哈希延迟被慢盘放大。
3. **目录扫描扇出**：无界并发会为每个文件开一个任务，万级目录瞬时
   分配与句柄压力不可控。

## 备选方案

1. **固定 128 KiB 块**：与 Go 默认互通，但小文件（< 1 MiB）也被迫
   切成多块，索引/内容比难看；大文件块数过多。否决。
2. **纯按大小连续函数**（如平方根曲线）：平滑但档位不可解释，与
   Go 行为对照困难。否决。
3. **哈希走 tokio blocking 池**：零新增设施，但哈希批量任务与
   sled I/O 在池中互相排队（tokio blocking 池上限 512，大扫描时
   哈希可占满）。否决。
4. **采用**：文件大小四档分块 + 独立 rayon 池 + 批量派发 +
   目录扫描 `JoinSet` 有界并发。

## 决定

- **clamp 区间**：`scan_file` 把调用方传入的块大小压到
  `[MIN_BLOCK_SIZE=4 KiB, MAX_BLOCK_SIZE=16 MiB]`，防御性约束非法输入。
- **四档分级**（`optimal_block_size`）：

| 文件大小 | 块大小 |
|:---|:---|
| < 1 MiB | 4 KiB |
| < 100 MiB | 128 KiB |
| < 1 GiB | 256 KiB |
| ≥ 1 GiB | 1 MiB |

- **批量哈希**：读满 32 块（32 × 128 KiB ≈ 4 MiB）才整批送进 rayon
  池；批量太小则 `spawn_blocking` 调度开销占比高，太大则单批内存
  驻留时间长。
- **专用 rayon 池**：`num_cpus` 线程、命名 `hash-worker-*`，与 tokio
  blocking 池物理隔离——哈希吃满核时 sled I/O 仍有独立通道。
- **目录扫描并发**：`JoinSet` 上限 `num_cpus().max(2)`，文件走
  `scan_file`（按大小选块），目录/符号链接走 `quick_scan`。

## 理由

四档分级让常见文件（< 100 MiB）与 Go Syncthing 的 128 KiB 行为一致
（互通性），同时把索引体积在小文件端压下来、把块数在大文件端控制住。
批量 + 专用池是"CPU 密集与 I/O 密集分离"的最小实现（ponytail：
不自建线程池管理，用 rayon 的一次性池构造）。
