---
type: ADR
title: ADR-20261105-index-update-dispatch：IndexUpdate 发送门控与分片
description: 本地索引更新传播的两个不变量——初始 Index 门控、序列号填充、按编码体积分片——及 ClusterConfig 设备列表与零字节文件 block 的 BEP 约束
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-index-update-dispatch：IndexUpdate 发送门控与分片

> English summary: Local `IndexUpdate` propagation obeys two invariants.
> (1) An `IndexUpdate` for a folder must only be sent to a device after that
> device has received the initial `Index` for the same folder, and its
> `prev_sequence`/`last_sequence` fields must be filled correctly — sending
> earlier disconnects Syncthing-Fork on Android. (2) Large change sets are
> chunked by encoded size (≤1 MiB per chunk) so a single message stays well
> below the BEP limit and the peer's decode memory spike is bounded. Related
> BEP wire constraints: deleted files must carry empty block lists, zero-byte
> regular files must carry one empty block (SHA256 of ""), and a
> `ClusterConfig` folder device list must include the local device.

## 问题

`LocalIndexUpdated` 事件到达时，若直接向所有已连接设备转发
`IndexUpdate`，会出现两类互操作事故：

1. **对端在收到初始 `Index` 之前收到 `IndexUpdate`**：手机端
   Syncthing-Fork 直接断开连接（2026-06-16 实测）。序列号字段
   （`prev_sequence`/`last_sequence`）填错有同样后果。
2. **单个 `IndexUpdate` 接近 BEP 消息上限**：大变更集（批量写入、初始
   同步）会把消息推到协议上限附近，对端解码时出现内存尖峰，且更容易
   触发对端实现的消息大小防御。

## 决定

- **初始 Index 门控**：以 `indexed_folders_map: DashMap<(DeviceId, String), u64>`
  记录已向每个设备的每个文件夹发送过的最大序列号；条目不存在即跳过
  该设备的 `IndexUpdate`（`cmd/syncthing/src/tui/index_dispatcher.rs`）。
- **序列号语义**：只转发 `sequence > prev_sequence` 的文件；每个分片的
  `prev_sequence` 取上一分片最大序列号，`last_sequence` 取本分片最大
  序列号，均按升序排列保证分片边界连续。
- **按编码体积分片**：`build_index_update_chunks` 边编码边试探，超过
  `MAX_INDEX_UPDATE_BYTES`（1,000,000 字节）即切分；部分发送失败时
  剩余文件在下个事件或重连时重试。
- **BEP wire 不变量**（`cmd/syncthing/src/tui/bep_handler.rs` 与
  index_dispatcher 共用的防御逻辑）：
  - deleted 文件 block list 必须为空；
  - 零字节普通文件必须有一个空 block（hash = SHA256("")），否则旧 DB
    条目会触发对端 "file with empty block list" 协议错误；
  - `ClusterConfig` 的每个 folder devices 列表必须包含本地设备。

## 备选方案

- 不做门控、直接转发：被 Syncthing-Fork 断开的事实否决。
- 按文件数分片：编码后体积与文件数不成线性（块列表大小差异大），
  按字节数分片才能给出确定的消息上限。

## 理由

门控 + 序列号填充是与 Go Syncthing 线行为对齐的最小条件；1 MiB 分片
上限给 BEP 消息上限留出充足余量，同时把对端解码内存尖峰限制在常量
级。三个 wire 不变量是 BEP 协议的硬约束，任何生成索引路径都必须遵守。
