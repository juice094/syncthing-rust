---
type: ADR
title: ADR-20261105-12: 扫描变更判定（内容哈希优先）
description: has_file_changed 以块哈希为首要判据、元数据变更暂不传播的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-12: 扫描变更判定（内容哈希优先）

> English summary: `Scanner::has_file_changed` treats identical block hash
> lists as "unchanged" and does not propagate pure metadata (mtime /
> permission) changes, avoiding infinite version ping-pong when syncing
> across platforms with different mtime precision and permission bits.

## 问题

扫描器每次运行都要回答"这个文件变了吗"。跨平台同步场景下，mtime 精度
（Linux ns vs Windows FILETIME 100ns 截断）与权限位（`.stignore` 里
记的 0o644 vs 接收端 OS 默认值 0o660）天然带噪声。如果以元数据差异为
首要判据，每次扫描都会产出"变更"：版本递增 → 对端回拉 → 对端再扫描
又判变更 → 无限版本乒乓。两边内容相同却互相覆盖，还会吞掉真实编辑
（真实编辑的 mtime 恰好与噪声同量级时被淹没）。

## 备选方案

1. **元数据优先（mtime → 权限 → 内容）**：实现直觉，但跨平台噪声必然
   触发上述乒乓。
2. **内容哈希优先（块哈希列表一致 = 未变更）**：块列表一致则直接判
   "未变更"，不再比较 mtime/权限；不一致再依次比较大小、mtime、权限
   （此时至少有一项真实差异）。
3. **元数据 + 白名单化平台规则**：按 Go Syncthing 的平台规则显式处理
   metadata-only 更新（何时合法传播 mtime/权限变化）。正确但工作量大。

## 决定

采用方案 2，方案 3 作为升级路径：

- `has_file_changed` 第一步比较块哈希列表：长度一致且逐块哈希相等 →
  返回 `false`（未变更），**不再看 mtime/权限**。
- 块列表不一致时，依次比较大小、mtime、权限，任一不同即判变更。
- ponytail: 纯元数据变更（mtime/权限）**暂不传播**；升级路径是按 Go
  Syncthing 的平台规则显式处理 metadata-only 更新（方案 3）。

## 理由

内容哈希是跨平台唯一稳定的"变与不变"判据。方案 2 以最小的实现成本
消除版本乒乓这一正确性灾难；代价是 metadata-only 变更（如 `touch`、
`chmod`）不会同步——在"内容优先"的同步语义下这是可接受的上限，且
已在代码内以 `ponytail:` 注释标记。
