---
type: draft
status: draft
project: syncthing-rust
tags: [draft, rfc]
---

# RFC-001: `syncthing-sync::service::SyncService` 业务拆分

**状态**: Draft → Accepted (2026-05-13)  
**作者**: juice094 / AI  
**关联任务**: T2.3 (NEXT_STEPS_2026-05-13.md)  
**实现 commit**: 见本 RFC 落地后的 split commit

---

## 1. 背景

`crates/syncthing-sync/src/service/mod.rs` 当前 **695 行**，是工程 top-1 单文件。
随着 T2.6 修复（`add_folder` spawn loop）落地，service 已成为下列职责的混合体：

1. 服务构造与生命周期（new/with_*/start/stop/init_folders/start_folder_loops/...）
2. `SyncManager` trait 实现（CRUD + 设备 + 文件夹操作）
3. BEP 网络层回调（handle_index / handle_index_update / handle_block_request /
   generate_index_update）
4. `syncthing_core::traits::SyncModel` trait 实现（FFI 边界）

四种职责混在一个文件，导致：

- 单文件超过软阈值 600 行（已多次推迟拆分）
- 修改某一类逻辑（如网络层钩子）需要在同一文件中跨越 ~500 行
- 测试时心智负担高，IDE 折叠 view 难以聚焦
- 新人阅读路径不清晰

## 2. 目标

仅做**位置移动**，不改变：
- 任何 `pub` API 表面（`use syncthing_sync::SyncService` 路径不变）
- 任何运行时语义（仍是同一个 `SyncService` 实例承担四类职责）
- 任何 trait 实现（仍是 `impl SyncManager for SyncService` 等）
- 所有现有测试通过率（295/295 + 1 e2e）

## 3. 拆分方案

`crates/syncthing-sync/src/service/` 目录最终结构：

```
service/
├── mod.rs              # struct 定义 + pub use 重新导出（~80 行）
├── lifecycle.rs        # impl SyncService 中的构造/生命周期函数（~170 行）
├── sync_manager.rs     # impl SyncManager for SyncService（~210 行）
├── network_bridge.rs   # impl SyncService 中的 BEP 网络回调（~105 行）
├── sync_model.rs       # impl SyncModel for SyncService（~155 行）
└── tests.rs            # 既有测试，无改动
```

### 3.1 拆分细节

| 目标文件 | 包含函数 |
|---------|---------|
| `mod.rs` | `struct SyncService` + `FolderTaskHandles`（如有）+ `pub mod` 声明 |
| `lifecycle.rs` | `new`, `with_config`, `with_block_source`, `set_block_source`, `start`, `stop`, `run`, `db`, `events`, `init_folders`, `add_folder_internal`, `start_folder_loops`, `start_folder_internal` |
| `sync_manager.rs` | `impl SyncManager for SyncService { ... }`（所有 trait 方法） |
| `network_bridge.rs` | `handle_index`, `handle_index_update`, `handle_block_request`, `generate_index_update`, `get_folder_ids`, `get_folder`, `get_folder_completion` |
| `sync_model.rs` | `impl SyncModel for SyncService { ... }`（FFI 边界） |

### 3.2 模块可见性

```rust
// mod.rs
pub struct SyncService { /* fields */ }

mod lifecycle;        // 私有，函数在 impl SyncService 中
mod sync_manager;     // 私有，trait 实现自动可见
mod network_bridge;   // 私有
mod sync_model;       // 私有
#[cfg(test)] mod tests;
```

子模块通过 `impl SyncService { ... }` 或 `impl SomeTrait for SyncService { ... }`
向上贡献方法，对外仍是单一 `SyncService` 类型。

### 3.3 导入策略

每个子模块顶部 use 自身需要的依赖。重复 use 块代价 < 10 行，
不引入额外耦合。

## 4. 验证清单

拆分 commit 必须满足：

- [x] `cargo fmt --check` 通过
- [x] `cargo clippy --release --all-targets -- -D warnings -W clippy::await_holding_lock` 0 警告
- [x] `cargo test --workspace --lib --release` 295/295 通过
- [x] `cargo test --release -p syncthing --test e2e_sync` 1/1 通过
- [x] 每个新文件 `wc -l` ≤ 600
- [x] `git diff --stat` 仅展示 service/ 内文件移动 + 删行；无业务逻辑改动

## 5. 风险

| 风险 | 缓解 |
|------|------|
| pub use 路径泄漏导致依赖方编译失败 | 仅在 mod.rs 重新导出 SyncService 等顶层 API；不动 crate `lib.rs` |
| trait 实现分散到多个文件可能 IDE 跳转失败 | rust-analyzer 已稳定支持跨文件 impl；旧版 RA 用户接受退化 |
| 测试文件路径变更影响 CI 缓存 | 不动 tests.rs |

## 6. 后续

拆分完成后，本 RFC 移到 `docs/design/RFC-001-service-split.md` 归档。

未来可继续抽：
- `network_bridge.rs` 进一步抽到 `crates/syncthing-net` 侧（需要 trait 升级）
- `sync_model.rs` 进一步抽到 `crates/syncthing-core` 侧（FFI 边界）

但本 RFC 范围仅限当前 crate 内位置移动。
