---
type: ADR
title: ADR-20261105-01: Pull 删除防护配额
description: 单次 pull 循环的远程删除配额与健康告警阈值的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-01: Pull 删除防护配额

> English summary: Per-pull quota protecting the local folder from anomalous mass
> deletions caused by a stale or corrupt peer index; a NO_SUCH_FILE storm also
> raises a stale-index health warning.

## 问题

对端索引如果陈旧或损坏，可能在一次 pull 中下发大量远程删除，静默清空本地
文件夹。2026-07-20 曾发生此类事故：114 个文件被一次性删除。需要在拉取循环
中内置一道防线，既放行正常删除，又在删除量异常时拒绝整个删除批次。

## 备选方案

1. 固定阈值（如"一次最多删 N 个"）：小文件夹（如共 10 个文件）会被误伤——
   正常重构也可能一次删除超过固定阈值。
2. 纯比例阈值（删除数 > 本地文件数 × 比例）：小文件夹下比例配额过低，
   同样误伤正常删除。
3. 比例 + 下限的组合配额：拒绝条件为
   `删除数 > max(PULL_DELETE_QUOTA_MIN, 本地文件数 × PULL_DELETE_QUOTA_RATIO)`。

## 决定

采用方案 3，常量定义于 `syncthing-core::constants`：

- `PULL_DELETE_QUOTA_MIN = 20`：一次 pull 循环允许应用的远程删除数至少放行
  此值，避免小文件夹因比例配额误伤正常删除。
- `PULL_DELETE_QUOTA_RATIO = 0.25`：删除数超过
  `max(PULL_DELETE_QUOTA_MIN, 本地文件数 × 比例)` 时判定为异常批量删除，
  拒绝整个删除批次（下载/修改动作不受影响）。
- `STALE_INDEX_WARN_THRESHOLD = 10`：单次 pull 中 BEP error code 2
  （NO_SUCH_FILE）响应数超过此值时，判定对端索引可能陈旧/损坏并输出健康
  告警。大量 NO_SUCH_FILE 正是 2026-07-20 事故的前兆信号。

## 理由

组合配额对小文件夹与大文件夹都安全：下限兜底小文件夹，比例兜大文件夹。
告警阈值独立于拒绝逻辑——拒绝保护本地数据，告警暴露对端健康度，两者互补。
该防线是 2026-07-20 事故（114 文件被删）的直接教训，不可移除。
