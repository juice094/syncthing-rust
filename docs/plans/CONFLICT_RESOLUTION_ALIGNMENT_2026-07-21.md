---
type: plan
status: active
project: syncthing-rust
date: 2026-07-21
tags: [plan, conflict, vector-clock, go-alignment]
---

# 冲突死循环检查报告与修复计划（对齐 Go 官方实现）

> **来源**：下游会话转述——五个修复（幻影删除、last_pong 未更新、僵尸会话拒绝重连、权限/mtime 噪声版本乒乓、.stversions 泄漏）已提交本地 main（`e2cf6fa`~`886ccee`，已随本文件推送）；残余问题为**分叉版本向量的冲突死循环**（双端同文件独立谱系时反复回拉覆盖）。
> **对照基准**：`dev/syncthing-go` 官方实现（`lib/protocol/vector.go`、`bep_fileinfo.go`、`lib/model/folder_sendrecv.go`）。

---

## 一、Go 官方的完整冲突机制（三个零件，缺一不可）

### 1. 内容级冲突精化 — `FileInfo.InConflictWith`（bep_fileinfo.go:188）

冲突判定**不是**纯向量并发。先 `GreaterEqual` 过滤；再比较内容哈希：新文件的 `PreviousBlocksHash` 若等于旧文件的 `BlocksHash`（新内容基于旧内容演进），即使向量并发也**不算冲突**。这挡住了 mtime/权限噪声造成的伪冲突。

### 2. 确定性胜者仲裁 — `FileInfo.WinsConflict`（bep_fileinfo.go:210）

并发冲突的胜者是**双方独立计算必得同一结果**的：

1. 仅一方 invalid → invalid 输；
2. **mtime 新者胜**；
3. mtime 相等 → 版本向量中设备 ID 定序决胜（`ConcurrentGreater`：Go 的 `Vector.Compare` 是五值——Equal/Greater/Lesser/**ConcurrentGreater**/**ConcurrentLesser**，并发也有方向）。

因为双端选的是同一个胜者，交换一次索引后两端向量即相等，**收敛，不循环**。

### 3. 败者留证 — `moveForConflict`（folder_sendrecv.go:1865-1897，命名函数 :2220）

败者本地文件改名为 `原名.sync-conflict-YYYYMMDD-HHMMSS-<设备短ID>.ext`（冲突副本自身再冲突时不重复复制）；冲突副本作为新文件进入正常索引同步；胜者版本原样落 DB。

---

## 二、我方现状与死循环根因

| 零件 | Go | syncthing-rust 现状 | 差距 |
|:---|:---|:---|:---|
| 冲突判定 | 向量并发 + 内容哈希精化 | 纯向量并发（`conflict_resolver::is_conflict`） | **缺内容精化** → 噪声伪冲突 |
| 胜者仲裁 | 确定性（mtime → 设备 ID 定序） | **无仲裁，一律取远程**（`resolve_conflict` RenameBoth/Merge 均接受 remote） | **死循环根源** |
| 比较代数 | 五值（并发有方向） | 四值（Conflict 无方向） | 缺 tiebreak 基础设施 |
| 败者留证 | `.sync-conflict-时间戳-设备短ID` | `.sync-conflict-时间戳-local`（`conflict_resolver.rs:160` 字面量 `"local"`，注释自认未用设备 ID） | 命名不含来源设备 |

**死循环机制**（独立谱系场景）：A 有 X `{A:5}`、B 有 X `{B:5}`（并发）。双端同时 pull：A 取 B 的 `{B:5}`，B 取 A 的 `{A:5}`——**内容互换**。下次索引交换：B 发现 A 的 `{B:5}` 与自己的 `{A:5}` 仍并发 → 再冲突 → 再互换 → **无限回拉覆盖**，冲突副本成堆。缺的就是 Go 零件 2：双端必须收敛到同一个胜者。

---

## 三、修复计划

### Phase 1：确定性胜者仲裁（根因修复，P0）

1. `syncthing-core::Vector` 增加**带方向的并发比较**：`compare` 返回扩展为五值（Equal/Greater/Less/ConcurrentGreater/ConcurrentLesser）；并发方向按首个分歧计数器对的 (ID, Value) 字典序判定，与 Go `Vector.Compare` 语义对齐。保留四值 `compare` 作为兼容包装或全量替换调用点（index_handler/conflict_resolver 共 3 处）。
2. `conflict_resolver` 新增 `wins_conflict(local, remote) -> bool`，语义与 Go 一致：invalid 输 → mtime 新者胜 → 相等则按并发方向决胜。
3. `resolve_conflict` 改为：先判 `is_conflict`，再仲裁——**本地胜 → 保留本地（远程版本被支配，无需动作）；远程胜 → 本地改名留证后接受远程**。双端同胜者 → 交换即收敛。
4. 冲突文件名带上设备短 ID：`{name}.sync-conflict-{YYYYMMDD-HHMMSS}-{local_short_id}{ext}`，与 Go 命名兼容（Go 端会将其识别为冲突副本）。

### Phase 2：内容级冲突精化（P1，消除噪声伪冲突）

`is_conflict` 增加短路：向量并发但 `remote.base_version == Some(local.blocks_hash)`（远端基于本地内容演进）或双端 `blocks_hash` 相等 → 非冲突，直接按向量大小定走向。我方 `FileInfo` 已有 `blocks_hash`/`base_version` 字段，只需在 index 收发时填充校验。

### Phase 3：回归测试（P0）

- **双端同时冲突收敛**（核心）：构造 A/B 两端同文件独立谱系，模拟互收索引 → 断言双端收敛到同一胜者、同一版本，且第二轮不再有冲突动作（死循环死亡证明）。
- mtime 决胜、设备 ID tiebreak 决胜各一例。
- 同内容并发向量 → 无冲突动作。
- 冲突文件名包含设备短 ID。

### 验收标准

- 下游 `peer_node` 双端独立谱系场景不再出现反复回拉；冲突副本仅产生一份。
- 与 Go 客户端互发并发冲突时，双方收敛到同一个 `.sync-conflict-*` 结果（命名兼容）。

### 明确不做

- 不改 Merge（文本三路合并）路径的语义，仅使其胜者方向与仲裁一致。
- 不引入 `PreviousBlocksHash` wire 字段新增——优先复用现有 `base_version`/`blocks_hash`；若 wire 兼容需要再评估。

---

*依据：Go 源码 `lib/protocol/vector.go` / `bep_fileinfo.go:188-227` / `lib/model/folder_sendrecv.go:1865-1897,2220-2231`；我方 `crates/syncthing-sync/src/conflict_resolver.rs` / `index_handler.rs`。*
