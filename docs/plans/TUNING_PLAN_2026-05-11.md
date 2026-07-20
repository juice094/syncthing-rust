---
type: plan
status: active
project: syncthing-rust
date: 2026-05-11
tags: [plan, roadmap]
---

# 调优计划 · syncthing-rust（2026-05-11）

> **定位**：在 [`POST_V0_2_0_ROADMAP.md`](../archive/plans/POST_V0_2_0_ROADMAP.md) 之外的 **横向调优补充**。本计划聚焦"功能已完成的代码如何变快、变稳、变干净"，不引入新功能。
> **制定原则**：先测量再优化（measure-first）；不破坏 BEP 协议兼容性；遵守 AGENTS.md §代码健康与架构约束硬性红线。
> **制定日期**：2026-05-11
> **维护者**：juice094

---

## 〇、前置体检结论（2026-05-11 自动审计）

| 维度 | 指标 | 备注 |
|------|------|------|
| 源代码行数 | 31,046 行 Rust | 152 文件 |
| 注释率 | 约 4.5% | 偏低，关键路径需补 |
| 测试代码 | 1,218 行内嵌；总 309 通过 | 0 失败 / 4 ignored |
| 静态检查 | 0 clippy warnings | baseline |
| 大文件（>600 行软上限） | **12 个** | 触发 AGENTS.md §2 红线 |
| unwrap/expect 出现 | 718 处（含测试） | 非测试代码待逐个审计 |
| tokio::spawn | 91 处 | 任务句柄回收策略待审 |
| .clone() | 371 处 | 部分热路径有省略空间 |
| target/release 体积 | 1.9 GB | 缺少 cargo sweep 配置 |
| daemon.log 单次 | 1.2 MB | 无 rotation/cap，长跑风险 |
| **基准测试基础设施** | **已建** ✅ | criterion + bench 可编译；report 路径可用 |
| **rayon 并行哈希** | **已启用** ✅ | scanner 已使用 rayon 池（T-B1） |
| **MAX_BEP_MESSAGE_SIZE** | **64 MiB** ✅ | 已从 128 MiB 收紧（T-D4） |
| **日志滚动** | **tracing-appender 集成** ✅ | daemon 日志按日轮转，保留 7 天（T-F3） |
| **release-thin profile** | **已建** ✅ | 开发期构建用（T-G1） |

> 粗体项中 criterion + rayon 已落实；T-B2/T-C 等待下一轮执行。

---

## 一、优先级矩阵（P0~P5 对齐项目惯例）

| 任务 | 风险 | 工作量 | 优先级 | 依据 |
|------|------|--------|--------|------|
| **T-A 测量基建** | 中（无 baseline 调优无效） | 小（0.5~1 天） | **P0** | 任何性能改动必须可量化 |
| **T-F 稳定性长跑** | 高（72h 未跑等于赌） | 中（执行 3~5 天） | **P0** | 同 POST_V0_2_0 P0 |
| **T-B Scanner 并行哈希** | 低（局部纯函数改造） | 中（1~2 天） | **P1** | 大文件首次扫描核心瓶颈 |
| **T-C 存储热路径** | 中（涉及 FileSystemDatabase） | 中（1~2 天） | **P1** | 每文件一次 JSON 落盘是 O(N) 系统调用 |
| **T-D 连接/会话剪枝** | 中（BEP 关键路径） | 中（2 天） | **P2** | 减小 DoS 面 + 减少分配 |
| **T-E 架构债务收敛** | 低（重构无功能改动） | 大（3~5 天分批） | **P2** | 12 个超大文件 + 双 SyncModel |
| **T-G 构建与 CI** | 低 | 小 | **P3** | 改善单人维护体验 |

---

## 二、T-A 测量基建（P0，前置）—— ✅ 已完成（2026-05-11）

> 没有 baseline 的"优化"等于赌博。先用 0.5~1 天把测量基建打底。

### T-A1 criterion 基准 ✅
- `crates/bep-protocol/benches/encode_decode.rs`：Hello / Index(1k 文件) / ClusterConfig / Request-Response roundtrip
- `crates/syncthing-fs/benches/scanner.rs`：扫描 1 KB / 1 MB / 1 GB 文件；扫描含 10k 文件的目录
- `crates/syncthing-sync/benches/puller.rs`：MockBlockSource 拉取 100 MB / 1000 块 的并发吞吐
- 验收：`cargo bench --workspace` 可重复运行；`target/criterion/` 输出 HTML
- **状态**：`cargo bench --no-run` 编译通过；encode_decode / scanner / puller 三 bench 就绪。

### T-A2 flamegraph + dhat 文档化 ✅
- `docs/design/PROFILING.md`（新增）：Windows 下 `cargo flamegraph` 与 `dhat-rs` 集成命令
- `scripts/profile.ps1`：一键采集 BEP 握手 + 全量扫描的 flamegraph
- **状态**：文件已落地。

### T-A3 metrics.rs CSV dump ✅
- `crates/syncthing-net/src/metrics.rs` 已存在 `flush_to_csv`（commit 前已有但缺调用点）
- stress_test 集成：BEP 字节、块请求、心跳延迟落 CSV —— 待 stress_test 运行时显式调用 `metrics::global().flush_to_csv(...)`
- **状态**：API 已存在，集成只需在 stress_test.rs 中加一行调用。

---

## 三、T-B Scanner 并行哈希（P1）—— T-B1 已完成（2026-05-11）

### 现状（瓶颈点）
`crates/syncthing-fs/src/scanner.rs` `compute_block_hashes`：
1. `file.read(&mut buffer).await` —— 异步 IO
2. `sha2::Sha256::digest(&buffer[..n])` —— **CPU 绑定，阻塞 tokio worker**
3. push `Vec<BlockInfo>`

128 KB block × N，单文件 1 GB 哈希耗时 ≈ 3~5 秒在 SSD 上。CPU 在 sha2 期间满载，但只用 1 核。

### T-B1 哈希工作池化 ✅
- `compute_block_hashes` 改造：tokio 异步读 → `tokio::task::spawn_blocking` + `rayon::ThreadPool` 哈希 → 汇总有序 `Vec`
- 避免 `tokio::spawn_blocking` 在 sha2 上占满 blocking-pool（默认 512）
- 新增依赖 `rayon = "1.10"`、`num_cpus = "1.16"` 到 `crates/syncthing-fs/Cargo.toml`
- **状态**：已落地。scanner 66 测试 + 5 doc-tests 全通过。

### T-B2 目录扫描并发 ✅
`scan_directory` 原 `for entry { scan_file().await }` 串行。已改为 `tokio::task::JoinSet` + 流控（max_concurrent = num_cpus::get().max(2)）：
- 顺序无关；8 核 CPU 100K 小文件扫描预期提速 4~6×
- **状态**：已落地。scanner 测试全通过。

### T-B3 复用 buffer ⏳
当前每次 `scan_file` 都 `vec![0u8; block_size]`。改 `BytesMut` 或 ThreadLocal 复用。减少分配。

### T-B4 BLAKE3 评估（仅本地缓存）⏳
- 协议层 SHA-256 不可换（BEP 兼容）
- 本地块缓存命中检查可用 BLAKE3（3~5× 快）
- **决策点**：评估收益 vs 复杂度

### 验收（T-B）
- [x] `cargo check --workspace --all-targets` 0 warnings
- [x] 309 测试通过 + 新增并发场景测试
- [ ] `cargo bench scanner_1gb` **>= 2×** 加速（基线由 T-A1 建立）— 待 bench 实际跑

---

## 四、T-C 存储热路径（P1）

### 现状（瓶颈点）
`crates/syncthing-sync/src/database.rs::FileSystemDatabase`：
- 每文件一份 `<name>.json` 落盘（`update_file:230-249`）
- `get_folder_files:208-228` 扫描目录读所有 JSON —— **O(N) 同步 IO**
- `update_files:251-256` 串行调用 `update_file` —— **无批量化**
- 内存缓存 `DashMap<folder_id, Vec<FileInfo>>` 无 size cap —— 长跑泄漏风险

注：项目已有 `crates/syncthing-db`（sled 后端）但 sync 层仍用 FileSystem 实现 —— 历史债。

### T-C1 收敛到 syncthing-db
- `LocalDatabase` trait 接口保持不变
- 新增 `SledLocalDatabase` 实现，复用 `syncthing-db::MetadataStore`
- `daemon_runner.rs` 切换默认实现；`FileSystemDatabase` 保留为调试模式 fallback
- **风险**：sled 数据目录与 JSON 共存期需迁移考量

### T-C2 批量化 update_files ✅
- 已改为 `tokio::task::JoinSet` 并发写入；`FileSystemDatabase` 新增 `#[derive(Clone)]`
- DashMap shard 锁天然保护同 folder 的并发缓存更新，无需额外同步
- **状态**：已落地。测试通过。

### T-C3 内存缓存上限 ✅
- `FileSystemDatabase` 新增 `cache_size: AtomicUsize` + `cache_cap: usize`（默认 100K）
- `update_file` 新增文件时计数；超限时调用 `evict_one_folder()` 随机驱逐整 folder
- `delete_file` 删除时递减计数，保持近似一致
- **状态**：已落地。测试通过。

### T-C4 sled flush 调优（依赖 T-C1）
- `sled::Config::flush_every_ms(1000)` → 5000 减少落盘
- `update_folder_index_meta` 显式 flush()；常规写入异步

### 验收（T-C）
- 1K 文件目录的 `get_folder_files` **< 50 ms**
- `update_files(100)` 批量 **< 单文件 update_file × 100 的 0.3 倍**
- 72h 压测内存 RSS 增长 **< 50%**

---

## 五、T-D 连接/会话剪枝（P2）—— T-D1 + T-D4 已完成

### T-D1 Hello 编码零拷贝 ✅
`crates/bep-protocol/src/messages.rs::Hello::encode_to_vec`：
- 当前 `BytesMut::new` → 写入 → `to_vec()` 拷贝
- **新增 `encode_to_bytes(&self) -> Bytes`**；`encode_to_vec` 委托 `encode_to_bytes().to_vec()` 保持兼容
- commit `78061b7` 已对 `encode_message` 做过类似优化，复用思路
- **状态**：已落地。bench 编译通过。

### T-D2 BepSession 热循环 instrument ✅
- `BepSessionMetrics` 新增 5 个 per-message-type 计数器：
  - `index_received` / `index_update_received` / `requests_received` / `responses_received` / `closes_received`
- `handle_message` 中各分支成功解码后自动累加
- REST API 暴露 —— 待后续 `/rest/system/status` 字段追加（非阻塞）
- **状态**：已落地。编译通过。

### T-D3 pending_responses 池化 ⏳
- `DashMap<i32, oneshot::Sender<Response>>` 每个 Request 一次分配
- 改 `slab::Slab` 复用槽位
- 收益：高频块请求（>1000 req/s）减少 1/3 分配

### T-D4 协议层参数收紧（防御） ✅
- `MAX_BEP_MESSAGE_SIZE = 128 MiB` 太宽松，攻击面大
- **已收紧至 64 MiB**（`crates/syncthing-net/src/connection.rs:35`）
- `connection_timeout = 120s` 在 relay 偏长，分级：直连 60s / relay 180s —— 待后续 commit

### 验收（T-D）
- BEP 编解码基准 **不退化**
- 新增 fuzz 用例覆盖收紧后边界
- 0 新增 clippy

---

## 六、T-E 架构债务收敛（P2，分批执行）

### 大文件清单（按收益降序）
| 文件 | 行数 | 拆分建议 |
|------|------|----------|
| `syncthing-fs/src/ignore.rs` | 904 | **可能直接删除**（标记 DO NOT USE） |
| `syncthing-core/src/types.rs` | 888 | `types/{device,file,folder,vector}.rs` |
| `syncthing-net/src/session.rs` | 874 | `state_machine + handler + metrics` |
| `bep-protocol/src/messages.rs` | 873 | `hello / cluster_config / index / request_response` |
| `syncthing-net/src/stun.rs` | 861 | `client / parser / coords` |
| `syncthing-net/src/connection.rs` | 709 | `stream + handshake + heartbeat` |
| `syncthing-db/src/metadata.rs` | 690 | `keys + ops + folder_stats` |
| `syncthing-sync/src/folder_model.rs` | 654 | `state + transition` |

### T-E1 优先拆分
- `syncthing-fs/src/ignore.rs`：先确认是否还在 build path 上；不在则**直接删除**（最大收益、零风险）
- `bep-protocol/src/messages.rs`：按消息类型拆，最易切割

### T-E2 双 SyncModel trait 收敛
- AGENTS.md §3 已指出
- 计划：sync 内部 trait 重命名 `SyncModelImpl`（`pub(crate)`），公共 API 统一用 core 版

### 验收（T-E）
- 12 个超大文件 → ≤ 4 个
- 309 测试不退化
- 0 公共 API 变更

---

## 七、T-F 稳定性长跑（P0，与现有路线图联动）

> 与 POST_V0_2_0_ROADMAP.md Phase B 重叠，**不重复制定**，仅追加调优配套项。

### T-F1 72h 压测执行（继承 POST_V0_2_0 P0）—— 基础设施就绪，可立即启动
- `cmd/syncthing/src/bin/stress_test.rs` 已强化：
  - **T-A3 hook**：每 10 分钟自动 `metrics::global().flush_to_csv()`
  - **内存采样**：`sysinfo` 采集 RSS（MB）写入报告 CSV
  - **变长负载**：注入文件大小轮换（1 KB / 64 KB / 1 MB / 10 MB），覆盖 block 哈希与传输全链路
  - **清理旧报告**：启动时删除上次 report + metrics.csv，避免数据混淆
- **启动命令**：`cargo run --release --bin syncthing -- stress-test --duration 72h`
- **格雷侧/Linux 部署**：复制 `target/release/syncthing.exe`（或 Linux 构建）+ 执行上述命令

### T-F2 unwrap/expect 审计 ⏳
- 当前 718 处（含测试）；目标 **非测试代码 < 50 处**
- 优先级：`crates/syncthing-net/` 与 `crates/syncthing-sync/` 是热路径
- 原则：错误向上传播用 `?`；不可恢复 panic 用 `expect("详细原因")` 而非裸 `unwrap()`

### T-F3 日志切片 ✅
- daemon.log 短跑 1.2 MB → 长跑 GB 级
- **已引入 `tracing-appender::rolling::Builder::daily()` + `max_log_files(7)`**
- 写入路径：`config_dir/logs/daemon.YYYY-MM-DD.log`
- 默认保留 7 天
- **状态**：`cmd/syncthing/src/main.rs` `Run` 命令已集成。Tui 模式保持原有 sink 行为。

---

## 八、T-G 构建与 CI（P3）

### T-G1 target 体积控制 ✅
- 1.9 GB target/release，定期 `cargo sweep -t 7`
- `.cargo/config.toml` 已创建：`[profile.release-thin]` `inherits="release"` `lto="thin"` `codegen-units = 16`
- 用法：`cargo build --profile release-thin`（编译快 2~3 倍）

### T-G2 criterion CI 回归 ⏳
- GitHub Actions 每次 push 跑核心 bench
- 超过 5% 退化报警（criterion-compare-action）

### T-G3 cargo-audit ✅
- `.cargo/audit.toml` 已存在（POST_V0_2_0 Phase A 完成）
- CI 建议改用 `cargo audit --no-fetch`

---

## 九、执行节奏建议

```
Week 1（5-day budget）
├── Day 1: T-A1 criterion + T-A3 metrics CSV
├── Day 2: T-B1 + T-B2 scanner 并行化（最大可见收益）
├── Day 3: T-C1 或 T-C2 二选一（视风险评估）
├── Day 4: T-D1 + T-D2 + T-D4 BEP 剪枝
└── Day 5: T-F1 启动 72h 压测

Week 2（按需）
├── T-E1 拆分大文件（连续 3~5 天，可中断）
├── T-F2 unwrap 审计（随手 PR，长尾）
└── T-F3 日志切片（1 小时）
```

---

## 十、不做项（明确冻结）

- ❄️ 替换 sled 为 redb/rocksdb —— ADR-002 已决策接受
- ❄️ SHA-256 换 BLAKE3 用于 BEP 协议 —— 破坏兼容性
- ❄️ async-std / smol 评估 —— tokio 已稳定
- ❄️ 重写 BEP 编解码用 prost —— 4-11 互通验证已通过，无收益
- ❄️ QUIC 传输 —— POST_V0_2_0 P5，待 TCP 完全稳定

---

## 十一、与现有计划关系

| 文档 | 关系 |
|------|------|
| [`POST_V0_2_0_ROADMAP.md`](../archive/plans/POST_V0_2_0_ROADMAP.md) | 本计划是其**横向补充**；T-F 共享 P0 72h 压测；不冲突 |
| [`PHASE3_PLAN.md`](./PHASE3_PLAN.md) | PHASE3.4 = T-F1，本计划提供测量基建辅助 |
| [`../../AGENTS.md`](../../AGENTS.md) §代码健康 | 本计划 T-E 直接落实 §2 文件 600 行红线 |

---

## 十二、验收门禁（整体）

- [ ] T-A 完成：`cargo bench --workspace` 可重复
- [ ] T-B 完成：1 GB 文件扫描 2× 加速
- [ ] T-C 完成：1K 文件目录 list 在 50 ms 内
- [ ] T-D 完成：BEP 编解码不退化
- [ ] T-E 完成：≤ 4 个文件超 600 行
- [ ] T-F 完成：72h 压测 0 panic / RSS 增长 < 50% / 重连成功率 > 95%
- [ ] 全部完成：`cargo test --workspace` 309+ 全通过；`cargo clippy` 0 warnings 保持
