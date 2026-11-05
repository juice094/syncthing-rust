---
type: ADR
title: ADR-20261105-14: MetadataStore 键布局与删除墓碑保留
description: syncthing-db 元数据键格式、删除文件以墓碑保留在索引内、folder stats 全量重算的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-14: MetadataStore 键布局与删除墓碑保留

> English summary: `MetadataStore` encodes every record as a flat sled key:
> file entries as `<folder_id>:<file_name>`, folder stats as
> `folder_stats:<folder_id>`, per-device files as
> `device/<device_id>/folder/<folder_id>/file/<file_name>`. Deleted files
> stay in the index as tombstones (serialized with the deleted flag, never
> physically removed by delta updates), folder stats are fully recalculated
> on every mutation, and folder indexes are returned sorted by sequence.

## 键布局

| 数据 | 键格式 | 访问方式 |
|:---|:---|:---|
| 文件条目 | `<folder_id>:<file_name>` | 点查 / `scan_prefix("<folder_id>:")` |
| 文件夹统计 | `folder_stats:<folder_id>` | 点查，缺省返回零值 |
| 设备文件 | `device/<device_id>/folder/<folder_id>/file/<file_name>` | 点查 + 前缀扫描 |
| 块数据 | `block/<hash>`（store.rs）/ `block:<hash>`（block_cache） | 点查 |

注：两个 BlockStore 实现（`store.rs` 的 `BlockStoreImpl` 与
`block_cache` 的 `CachedBlockStore`）当前使用不同的块键前缀，互不通读。
这是存量事实，统一前缀属于合并两个实现时的工作。

## 问题

1. **删除语义**：BEP 的索引是版本向量——"文件被删除"本身是一个必须
   广播给对端的索引版本。如果 delta 更新物理删掉记录，对端重连全量
   索引时就永远学不到"这个文件没了"，删除无法传播。
2. **统计一致性**：`file_count` / `total_bytes` / `block_count` 是
   REST 状态与配额判断的输入，必须与索引内容一致。
3. **枚举确定性**：`get_folder_index` 的返回顺序直接影响上层比对与
   测试稳定性。

## 备选方案

1. **删除即物理删除**：索引里没有墓碑，删除无法进入索引版本流。否决。
2. **独立墓碑区**：删除记录写进单独的 key 空间。多一次查询路径，而
   索引消费者本来就要按文件名遍历。否决。
3. **增量维护统计**（put/delete 时加减计数）：O(1) 但每次 delta 批量
   更新要逐条算去重块数（同一块被多文件引用时不能重复计数），复杂度
   转移到热路径。否决（见"理由"）。
4. **每次变更后全量重算（采用）**：任何 put/delete/update 之后
   `update_folder_stats` 重新扫描该 folder 全部条目，跳过
   `is_deleted()` 的文件，按块哈希去重计数。

## 决定

- **墓碑保留**：`update_index_delta` 对所有传入文件一律序列化写入，
  包括 `is_deleted()` 的文件；删除标记随 FileInfo 走，记录本身留在
  索引内。
- **统计全量重算**：一致性优先于写放大；重算在同一 `run_blocking`
  语义下完成，失败即整体报错。
- **顺序保证**：`get_folder_index` / `get_device_files` 返回前按
  `FileInfo.sequence` 排序。

## 理由

墓碑方案让删除与其他任何索引变更走同一条序列化/存储/广播路径，
不需要第二套机制（ponytail：删除不是特殊状态，只是 FileInfo 的一个
标志位）。全量重算把"统计与索引一致"变成构造性保证——不存在忘了
更新的计数器；代价是每次变更 O(folder 大小)，对万级文件的 folder
是可接受的上限。

> ponytail: 统计全量重算是 O(n) 写放大；升级路径是增量维护
> `(count, bytes, unique_block_set)` 并在 delta 批量更新时批量去重。
