---
type: ADR
title: ADR-20261105-16: .stignore 模式编译为单条正则与目录剪枝安全条件
description: IgnorePatterns 把 glob 模式编译成单条锚定正则、last-match-wins、无取反且无 (?d) 时才允许整目录剪枝的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-16: .stignore 模式编译为单条锚定正则与目录剪枝安全条件

> English summary: each ignore pattern is compiled into one anchored regex:
> `*` becomes `[^/]*`, `**` becomes `.*` (or `(?:.*/)*` when followed by
> `/`), `?` becomes `[^/]`, `.` and regex metacharacters are escaped, a
> non-root-only pattern gets a `(?:.*/)?` prefix and a directory-only
> pattern gets a `(?:/.*)?` suffix. Matching evaluates all patterns in
> definition order with last-match-wins (a `!` include pattern un-ignores).
> Directory pruning during scans is allowed only when no `!` and no `(?d)`
> patterns exist (`allows_skipping_ignored_dirs`).

## 问题

`.stignore` 的语法是类 gitignore 的 glob，但要满足三条硬约束：

1. **跨平台路径形态**：Windows 反斜杠分隔符必须在匹配前归一化为
   `/`，否则同一条规则在两端的判定不一致。
2. **语义等价于 Go Syncthing**：`*` 不跨 `/`、`**` 跨任意层级、
   `!` 取反、末尾 `/` 只匹配目录、`(?i)` 大小写不敏感、`(?d)` 允许
   删除被忽略目录——逐条偏差都会表现为与官方 daemon 互相同步时的
   忽略判定分歧。
3. **扫描性能**：万级文件的目录树不能为每个条目跑 O(规则数 × 回溯)
   的多模式匹配。

## 备选方案

1. **引入 `globset` / `ignore` crate**：成熟但新增依赖，且其 glob
   方言与 Syncthing 的 `(?d)`、`#include` 语义不完全对齐，需要对
   每个差异写桥接。依赖纪律（能不加就不加）下收益不够。
2. **逐模式循环 + 自带通配引擎**：手写匹配器要正确处理 `**` 的
   任意层级语义，状态机复杂度高于一次正则编译。
3. **每模式编译一条锚定正则（采用）**：`regex` crate 已在依赖内，
   编译期一次性完成，运行期每条规则一次 `is_match`。

## 决定

- **编译规则**（glob → regex，全部锚定 `^...$`）：

| glob 片段 | regex | 说明 |
|:---|:---|:---|
| `*` | `[^/]*` | 不跨目录层级 |
| `**` | `.*` | 跨任意字符 |
| `**/` | `(?:.*/)*` | 任意深度目录前缀 |
| `?` | `[^/]` | 单字符 |
| `.` `+` `(` `)` 等 | `\\.` `\\+` … | 原义转义 |
| 非 `/` 开头模式 | 前缀 `(?:.*/)?` | 可匹配任意层级 |
| 末尾 `/`（目录规则） | 后缀 `(?:/.*)?` | 目录及其内容 |
| 前缀 `(?i)` | 内联 flag | 大小写不敏感 |

- **判定语义**：按定义顺序逐条匹配，最后一条命中者生效；`!` 模式
  命中把状态翻回"不忽略"。
- **前缀解析**：`!`、`(?i)`、`(?d)` 任意顺序各至多一次；`/` 开头
  为仅根匹配；`\` 转义下一字符。
- **`#include` / `@include`**：递归加载，已加载集合判环，环即跳过。
- **剪枝安全**：仅当规则集中没有 `!` 取反且没有 `(?d)` 时
  （`allows_skipping_ignored_dirs`），扫描器才能整目录跳过被忽略
  目录——否则被忽略目录内可能有被 `!` 重新包含的文件，剪枝会漏扫。

## 理由

方案 3 把"理解 glob 语义"的全部复杂度压进编译期的一次性正则构造，
运行期复用高度优化的 `regex` 引擎；锚定 + 前缀/后缀片段的组合已覆盖
Go 实现的实测语义。剪枝条件是正确性闸门而非优化开关：漏判一个
`!` 规则等于把该文件永远排除在同步之外，比慢一个数量级更糟。
