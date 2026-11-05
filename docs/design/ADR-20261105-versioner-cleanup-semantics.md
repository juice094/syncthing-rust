---
type: ADR
title: ADR-20261105-versioner-cleanup-semantics：版本归档清理语义
description: Simple 在 archive 时即时裁剪、clean() 为 no-op；Staggered 的窗口表、maxAge 淘汰与清理不变量
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-versioner-cleanup-semantics：版本归档清理语义

> English summary: Both versioners archive replaced/deleted files into
> `.stversions` with a `name~YYYYMMDD-HHMMSS` timestamp. Simple keeps at most
> `keep` versions per file and prunes eagerly inside `archive()`, so its
> `clean()` is intentionally a no-op. Staggered keeps versions at decreasing
> frequency (30s / 1h / 1d / 1w windows up to `maxAge`, default 1 year) and
> does all pruning in `clean()`: first drop files older than `maxAge`, then
> per file group keep at most one version per window step, always preserving
> the oldest version of each run. Restore matches a version timestamp with a
> 2-second tolerance.

## 问题

Go syncthing 的 `lib/versioner` 提供多种版本策略，各自回答两个问题：
何时裁剪旧版本（写入路径还是独立清理路径），以及按什么规则决定
哪些版本过期。本实现需要为 Simple 与 Staggered 两个策略固定这些
语义，并保证 `Versioner` trait 的 `clean()` 契约对两者一致可调用。

## 备选方案

1. **统一在 `clean()` 里裁剪**：两个策略都把裁剪逻辑放进定期
   `clean()`，写入路径只做归档。优点是实现对称；缺点是 Simple 的
   `.stversions` 目录会在两次 clean 之间无限增长，违背「最多保留
   keep 个版本」的用户预期。
2. **Simple 即时裁剪 + Staggered 定期裁剪**（采纳）：Simple 在每次
   `archive()` 后立即裁剪到 `keep` 个，`clean()` 为 no-op；Staggered
   保留窗口语义必须依赖全局时间视角，只能在 `clean()` 中执行。

## 决定

- **Simple**：`archive()` → 归档 → `prune()` 删除超出 `keep` 的最旧
  版本。版本列表本身按时间新→旧排序（`list_versions_for`），因此
  `versions[keep..]` 即待删集合。`clean()` 不做任何事。
- **Staggered**：窗口表（与 Go `staggered.go` 对齐）：

  | 年龄段 | 步长 |
  |:---|:---|
  | 第 1 小时 | 30 秒 1 个版本 |
  | 第 1 天 | 1 小时 1 个版本 |
  | 前 30 天 | 1 天 1 个版本 |
  | 至 `maxAge`（默认 1 年） | 1 周 1 个版本 |

  `clean()` 分两步：先按 `maxAge` 直接淘汰过期文件；再按「原始文件名
  （`~` 前缀）分组」，对每组按年龄升序扫描，只保留与上一个保留版本
  相距 ≥ 当前窗口步长的版本。
- **不变量**：每组**最旧的一个版本永远保留**（`prev_age` 为 None 时
  直接保留并作为基准），保证任何文件至少有一个可回滚的历史点。
- **时序**：`get_versions()` 对 Staggered 返回**最旧在前**（窗口算法
  要求升序）；`list_versions_for()` 的原始返回是**最新在前**（REST 展示
  语义），Staggered 在使用前自行重排。
- **restore 容差**：`restore_by_timestamp` 以 2 秒容差匹配版本时间戳，
  因为归档文件名时间戳精度为秒，调用方传入的 `version_time` 可能来自
  毫秒级时钟。

## 理由

- Simple 的用户契约是「最多 keep 个」，即时裁剪是唯一能保证该上界的
  方案；放到 clean() 会使上界依赖清理周期。
- Staggered 的窗口判定需要「当前时间 + 全部历史版本」两个全局视角，
  在单次 `archive()` 时无法正确判定远处窗口的稀疏度，因此必须留在
  `clean()`。
- 始终保留最旧版本是 Go 版行为，防止「全部历史恰好落在同一窗口步长
  内」时被裁剪到无法回滚。

## 升级路径

- Simple 的 `clean()` 未来可扩展为基于 `CleanContext.now` 的全局
  清理（如修复归档残留的孤儿文件），不影响现有 archive 路径语义。
- `maxAge` 可通过 `VersioningConfig::Staggered` 的 `maxAge` 参数
  覆盖，未配置时取默认值 1 年。
