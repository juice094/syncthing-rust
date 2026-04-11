# Wave 2 里程碑报告

**日期**: 2026-04-09  
**分支/状态**: 主工作区 (syncthing-rust-rearch)

---

## 1. 本阶段目标回顾

Wave 2 的核心任务是补齐**同步引擎核心**：
- Delta Index（增量索引交换）
- 完整冲突解决流水线
- 完整 ignore 规则（因当前工作区无 `syncthing-fs`，基于子代理独立验证）

---

## 2. 完成项

### 2.1 Delta Index (`syncthing-sync`)
- **实现文件**: `crates/syncthing-sync/src/index.rs`（新建）
- **核心功能**:
  - 新增 `IndexID` 类型（`[u8; 8]`）与 `IndexDelta` 结构体（`syncthing-core/src/types.rs`）
  - `FileInfo.sequence` 统一从 `i64` 改为 `u64`，与 Go Syncthing 对齐
  - `IndexManager` 支持：
    - `get_index_delta(device)` — IndexID 匹配时返回 sequence 更大的文件，不匹配时返回 `None`（触发全量）
    - `update_index_id()` — 重置并生成新的随机 `IndexID`
    - `update_index()` / `update_index_delta()` — 分配递增 `sequence` 并持久化
  - 数据库层 (`database.rs`) 扩展 `LocalDatabase` trait：支持 folder index metadata 的读写（内存 + 文件系统 JSON）
- **测试覆盖**: 6 个新单元测试，全部通过

### 2.2 冲突解决流水线 (`syncthing-sync`)
- **实现文件**: `puller.rs`, `conflict_resolver.rs`
- **核心功能**:
  - 当 `ConflictResolution::Conflict` 时，自动创建物理冲突拷贝（`handle_conflict_copy`）
  - 版本向量合并（`merge_versions`）— 按 device 取最大 counter
  - 冲突文件名格式：`filename.sync-conflict-YYYYMMDD-HHMMSS-{short_id}.ext`
  - 修复 `last_writer_wins` 中比较 device ID 与文件名的 bug
- **测试覆盖**: 3 个冲突相关测试，全部通过

### 2.3 编译与集成修复
在 `cargo test --workspace` 验证过程中，发现并修复了以下**阻碍工作区编译**的问题：
1. **根 `Cargo.toml`**: 移除了误加入 workspace 的 `"Desktop/devbase"`
2. **`syncthing-net/Cargo.toml`**: 将 `iroh` 从 `default = ["iroh"]` 降级为 `default = []`，避免未完成的 iroh 集成阻塞默认编译
3. **`syncthing-net/src/lib.rs`**: 为 `IrohTransport` 的 `pub use` 补上了 `#[cfg(feature = "iroh")]`
4. **`dev/third_party/iroh/iroh-relay`**: 修复 `txt.txt_data()` → `txt.txt_data`（字段非方法）
5. **`dev/third_party/iroh/iroh`**: 修复 `packet.to_response()` → `packet.clone().into_response()` 等 hickory-proto API 变更导致的编译错误

> 说明：`dev/third_party` 为外部依赖/参考仓库，未纳入本仓库 Git 跟踪。

---

## 3. 测试验证结果

执行命令：
```bash
cargo test -p syncthing-core -p syncthing-sync -p syncthing-net -p bep-protocol
```

| Crate | Passed | Failed | Ignored |
|-------|--------|--------|---------|
| `bep-protocol` | 13 | 0 | 0 |
| `syncthing-core` | 12 | 0 | 0 |
| `syncthing-net` | 34 | 0 | 1 |
| `syncthing-sync` | 23 | 0 | 0 |

**总计：82 passed, 0 failed, 1 ignored**

---

## 4. 已知问题与待办

### 4.1 iroh 集成（搁置）
- **状态**: `pending`
- **原因**: `iroh::Endpoint` + `Router` 的编译问题已降级为可选 feature（`iroh` feature），但其内部测试 `endpoint_relay_connect_loop` 仍不稳定/失败。因 BEP 协议栈与 iroh 的 ed25519/BLAKE3 身份体系不兼容，需要额外的适配层。建议后续以更小范围（仅 `IrohTransport` 可选 feature）重试，或优先完善 TCP+TLS 栈。

### 4.2 syncthing-fs / syncthing-db / syncthing-api
- **状态**: 当前工作区目录 (`crates/`) 中**不存在**这三个 crate。
- **影响**: ignore 规则与数据库相关任务无法在此代码库内编译验证。FS ignore 规则的完成状态基于此前子代理在独立工作区内的汇报（63+5 测试通过）。

---

## 5. 下一阶段展望 (Wave 3)

根据 `THIRD_PARTY_REFERENCING_PLAN.md`，Wave 3 主题为**连接管理与运行时**，候选任务包括：

1. **网络变更重绑定 (Network Rebind)** — 借鉴 Tailscale `netmon` 模式，监听系统网络接口变化并触发 TCP 连接重拨
2. **地址质量评分 + 并行拨号** — 对多地址（relay/direct）进行 RTT 探测与优先级排序，实现快速路径选择
3. **Rust 版 `suture.Supervisor`** — 为 folder service、connection manager 等长生命周期 actor 提供自动重启监督树

---

## 6. Git 状态

本里程碑已通过初始 commit 保存至本地 Git 仓库（未包含 `dev/third_party` 与系统/用户目录噪音文件）。
