# Known Issues

> **维护原则**：发现的缺陷必须显式登记，避免误判项目成熟度。  
> **最后更新**：2026-05-13（T2.6 修复）

本文档列举当前已知未修复的功能性 / 行为性问题。  
**这些问题决定了项目目前的"事实可用性"边界**。

---

## ⚠️ 项目阶段定位（2026-05-13 post-T2.6）

| 维度 | 状态 |
|------|------|
| 代码完成度 | ~85%（MVP 全部 module 编译；端到端链路通） |
| 单元测试覆盖 | 295/295 通过 |
| 连接层稳定性 | ✅ 9h+ 压测验证（T-F1 死锁已修复） |
| **端到端同步** | ✅ **已修复**（T2.6，见 §2） |
| 跨版本互通 | ⚠️ 仅 2026-04-11 单次手工验证，无自动化 |
| 长跑（72h） | ⏳ 未完成（Windows 桌面休眠限制） |
| 生产就绪度 | **alpha，已可用于研究 / 测试，未到生产** |

类比：发动机 / 变速箱 / 车架 / 轮子都装好了，传动轴的卡扣插好了，可以踩油门跑。但还没拿到出厂检验报告（72h 长跑）和路况认证（跨版本互通）。

---

## §1. ClusterConfig 首次握手必定超时（中危）

**症状**：两节点同时拨号对方时（双向 `connect_to`），首轮 BEP session 在 10s 内都收不到对端 ClusterConfig，触发 timeout → 重连 → 第二轮成功。

**复现**：`cargo test --test e2e_sync -- --ignored` 日志最早 10s 段。

**根因（推测）**：
- 双向 `connect_to` 在毫秒级内发起两个 TCP 连接
- 两个 `BepSession::run()` 异步启动，各自发送 ClusterConfig
- 由于 race resolution 机制立刻 close 一组连接，对方还没来得及读 ClusterConfig
- BepSession 的 ClusterConfig 等待逻辑硬编码 10s（`crates/syncthing-net/src/session/mod.rs`）

**影响**：
- 用户体感"启动后约 12 秒才真正连上"
- 长跑场景每次 process 重启都浪费 10s
- 不影响最终连通性（重连机制兜底）

**修复方向**：
- A. 在 race resolution 中转移已发送/已接收的 ClusterConfig 缓冲
- B. 推迟 BepSession::run() 启动直到 race resolution 稳定（~100ms 延迟）
- C. 缩短超时到 3s + 立刻重试

**追踪**：`docs/plans/NEXT_STEPS_2026-05-13.md` §T3 候选

---

## §2. 端到端文件同步未完成（**已修复 ✅** T2.6）

> **历史记录**：2026-05-13 早盘标定为 P0 阻断；同日晚 T2.6 定位并修复。

**症状（修复前）**：两节点连接成功 → ClusterConfig 交换 → Index 包含 1 文件 → 接收端日志显示  
`New file from remote file=hello.txt`  
**但之后没有任何 Block Request / Response 事件**，文件 45s 内不出现在接收端。

**实际根因**：
`SyncManager::add_folder`（`crates/syncthing-sync/src/service/mod.rs`）只调用 `add_folder_internal`（创建 `FolderModel`），但 **没有调用 `start_folder_internal`** 来 spawn 该文件夹的 scan/pull/watcher 三个循环。

具体后果链：
1. `start_folder_loops()` 只在 `SyncManager::start()` 时遍历当时已存在的 folders
2. 测试 / REST API / TUI 在运行时通过 `add_folder` 添加的文件夹 → FolderModel 存在但**没有任何任务**
3. 远程 Index 到达 → `index_handler` → `folder_model.handle_remote_index` → `pull_notify.notify_one()`
4. 但 `pull_loop` 任务从未 spawn → 没人在 `pull_notify.notified().await` 上等 → **通知静默丢失**
5. → `puller.pull_folder` 永不调用 → `BlockSource::request_block` 永不调用

**这不仅影响测试**：生产中通过 REST API `POST /rest/config/folders` 在运行时添加文件夹也会复现同样问题——重启进程前文件夹永不同步。

**修复**：
```rust
// service/mod.rs::SyncManager::add_folder (commit XXX, 2026-05-13)
async fn add_folder(&self, folder: Folder) -> Result<()> {
    // ...
    let folder_id = folder.id.clone();
    self.add_folder_internal(folder).await?;
    self.start_folder_internal(&folder_id).await?;  // ← NEW
    Ok(())
}
```

`start_folder_internal` 在 line 176 已经做了 `folder_tasks.contains_key()` 幂等检查，无条件调用安全。

**验证**：
```bash
cargo test --release -p syncthing --test e2e_sync test_two_node_single_file_sync -- --nocapture
# 12.18s, 1 passed
```

日志可观察到完整链路：
```
Received full index folder=e2e file_count=1
Starting folder pull file_count=1
Received Response id=1 code=0 data_len=54   ← Block 传输!
File download completed file=hello.txt
```

**追踪**：
- 修复 commit：见本次 commit
- 历史诊断测试：`cmd/syncthing/tests/e2e_sync.rs`（已解除 `#[ignore]`）
- 报告：`docs/reports/STRESS_TEST_REPORT_2026-05-13.md` §6

---

## §3. 72h 压测在 Windows 桌面不可行（中危，环境限制）

**症状**：Windows 桌面盖子合上 → S3 sleep → nohup 子进程被回收。  
**实测**：2026-05-12 启动的 72h 压测在 T+9h11m 进程消失（详见 `docs/reports/STRESS_TEST_PARTIAL_2026-05-12_to_05-13.md`）。

**根因**：Windows 桌面环境与无人值守长跑不兼容（与代码无关）。

**修复方向**：迁移到 Linux/WSL2/VPS 重跑（NEXT_STEPS T2.4）。

---

## §4. `TestNode` 默认 `rescan_interval_secs = 3600`（低危，文档缺失）

**症状**：测试中创建文件后 1 小时内不会被自动 scan，必须手动 `sync_service.scan_folder()`。

**影响**：编写测试时容易踩坑，误以为 sync 链路坏了，其实是 scanner 没启动。

**修复方向**：
- A. 文档化"测试中必须手动 scan_folder"
- B. `TestNode::new` 默认使用更短的 rescan 间隔（如 5s）

**追踪**：NEXT_STEPS T2.5 后续

---

## §5. CSV 时间戳格式（已修复 ✅）

**症状**：stress_test 输出 `20585T05:07:55Z`（broken format）。  
**修复**：`38fb07f feat(T2.2)`（chrono ISO 8601）。

---

## §6. stress_test `rss_mb` 恒为 0（已修复 ✅）

**症状**：monitor CSV `rss_mb` 列全 0。  
**修复**：T-F1 周期内 patch（process name fix）。本 commit `38fb07f` 仍未带新 binary，下次重跑生效。

---

## 路线图影响

按本文档现状（T2.6 修复后）：

| v0.X.Y | 必须包含 |
|--------|---------|
| **v0.2.5（patch）** | ✅ §2 已修复 — 准备发布 |
| **v0.3.0** | §1 ClusterConfig race + §4 TestNode 文档 + Linux 72h（§3） |
| **v0.4.0** | 跨版本互通自动化 + GUI / Web UI |

v0.3.0 路线图（`NEXT_STEPS_2026-05-13.md`）中 T2.6 已完成。

---

## 协作约定

新发现的缺陷请按以下结构补充到本文档：

```
## §N. <一句话症状>（严重程度）

**症状**：观察到的行为
**复现**：命令或步骤
**根因（推测）**：定位
**影响**：用户视角
**修复方向**：A/B/C 选项
**追踪**：测试文件 / issue / commit
```

不要直接在代码中 `#[ignore]` 而不在此文档登记。
