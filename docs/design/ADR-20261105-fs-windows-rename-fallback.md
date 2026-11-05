---
type: ADR
title: ADR-20261105-15: rename 跨设备与被占用目标的回退策略
description: NativeFileSystem::rename 对 EXDEV / ERROR_NOT_SAME_DEVICE 走 copy+delete、对 SHARING_VIOLATION / ACCESS_DENIED 走 remove+rename 的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-15: rename 跨设备与被占用目标的回退策略

> English summary: `NativeFileSystem::rename` first tries the atomic
> `fs::rename`. On cross-device errors (unix `EXDEV`, Windows
> `ERROR_NOT_SAME_DEVICE` = 17) it falls back to copy+delete; on Windows
> `ERROR_SHARING_VIOLATION` (32) or `ERROR_ACCESS_DENIED` (5) — the target
> is locked by another process — it removes the target and retries the
> rename, emulating Go Syncthing's move-by-replace behavior.

## 问题

同步引擎的"接收文件 → 临时名 → rename 就位"链路依赖 rename 接近
原子且跨平台一致。但两个场景会让裸 `fs::rename` 失败：

1. **跨设备移动**：源在临时目录、目标在 folder 根，两者可能不在同一
   卷（Windows 上 `temp` 与数据分区分离很常见）。unix 报 `EXDEV`，
   Windows 报 `ERROR_NOT_SAME_DEVICE (17)`。
2. **目标被占用**：Windows 上索引器、杀毒软件或预览缩略图会短时间
   锁住目标路径，rename 报 `ERROR_SHARING_VIOLATION (32)` 或
   `ERROR_ACCESS_DENIED (5)`。Go Syncthing 在同场景下用
   remove+rename 直接覆盖。

## 备选方案

1. **只报错误交给上层重试**：正确但用户体验差——杀毒软件持锁是
   秒级的，重试风暴会拖慢整个 folder。
2. **一律 copy+delete**：跨设备成立，但本地同卷移动也退化为全量
   拷贝，丢原子性且 I/O 翻倍。
3. **硬链接 + 删源**：跨卷不成立（硬链接不能跨设备），否决。
4. **分级回退（采用）**：原子 rename 优先；按 errno 精确分流。

## 决定（决策表）

| 平台 | errno | 含义 | 回退 |
|:---|:---|:---|:---|
| unix | `EXDEV` | 跨设备 | `copy_and_delete` |
| Windows | 17 (`ERROR_NOT_SAME_DEVICE`) | 跨设备 | `copy_and_delete` |
| Windows | 32 (`ERROR_SHARING_VIOLATION`) | 目标被锁 | 删目标后重试 rename |
| Windows | 5 (`ERROR_ACCESS_DENIED`) | 目标被拒 | 删目标后重试 rename |
| 其他 | — | — | 原样返回 `Io` 错误 |

前提：rename 前已为目标创建父目录，因此回退路径不再处理目录缺失。

## 理由

方案 4 让常见路径（同卷移动）保持 O(1) 原子操作，只在 errno 明确
指向"rename 在此环境不成立"时才付拷贝代价；Windows 锁场景对齐 Go
实现的行为（覆盖被锁目标而非失败），保证与官方 daemon 线行为互通
时的语义一致。

> ponytail: errno 魔数以注释位置就近解释，本表是权威出处；
> 升级路径是引入 `winapi`/`libc` 的具名常量替代字面量。
