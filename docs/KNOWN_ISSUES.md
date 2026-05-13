# Known Issues

> **维护原则**：发现的缺陷必须显式登记，避免误判项目成熟度。  
> **最后更新**：2026-05-13

本文档列举当前已知未修复的功能性 / 行为性问题。  
**这些问题决定了项目目前的"事实可用性"边界**。

---

## ⚠️ 项目阶段定位（2026-05-13）

| 维度 | 状态 |
|------|------|
| 代码完成度 | ~80%（MVP 全部 module 编译） |
| 单元测试覆盖 | 295/295 通过 |
| 连接层稳定性 | ✅ 9h+ 压测验证（T-F1 死锁已修复） |
| **端到端同步** | ❌ **失败**（见 §2） |
| 跨版本互通 | ⚠️ 仅 2026-04-11 单次手工验证，无自动化 |
| 长跑（72h） | ⏳ 未完成（Windows 桌面休眠 + harness 缺陷） |
| 生产就绪度 | **不可用于生产** |

类比：发动机 / 变速箱 / 车架 / 轮子都装好了，但传动轴某节有问题，踩油门发动机转、轮子不转。

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

## §2. 端到端文件同步未完成（**P0 关键缺陷**）

**症状**：两节点连接成功 → ClusterConfig 交换 → Index 包含 1 文件 → 接收端日志显示  
`New file from remote file=hello.txt`  
**但之后没有任何 Block Request / Response 事件**，文件 45s 内不出现在接收端。

**复现**：
```bash
cargo test --test e2e_sync -- --ignored --nocapture
# RUST_LOG=info,syncthing_sync=debug 可看到详细链路
```

**已确认正常的环节**：
- ✅ `TestNode::install_bep_bridge`（T2.5）正确注册 `on_connected`
- ✅ BepSession::run() 启动并进入 steady-state
- ✅ ClusterConfig 双向发送 / 接收
- ✅ Index 双向发送，Sender 端正确报告 1 file
- ✅ Receiver 端 index_handler 接收到 index 并识别 "New file from remote"

**断链位置（疑似）**：
```
index_handler.rs::on_index()
    ↓ identifies need_files
    ↓ ??? — should trigger folder_model / puller
    ↓
puller.rs::pull_file()  ← never called?
    ↓
BlockSource::request_block()  ← never called
```

**影响**：
- 项目**核心承诺**（文件同步）目前**不工作**
- 9h+ 压测的"稳定性"实际只是连接层稳定，sync 从未跑起来过
- 任何生产部署 = 文件不会同步

**修复优先级**：**P0 — 阻断 v0.3.0 一切其他增强**

**追踪**：
- 测试：`cmd/syncthing/tests/e2e_sync.rs`（已 `#[ignore]`）
- 报告：`docs/reports/STRESS_TEST_REPORT_2026-05-13.md` §6

**下一步行动（"B 路径"）**：
1. 阅读 `crates/syncthing-sync/src/index_handler.rs`
2. 阅读 `crates/syncthing-sync/src/folder_model/mod.rs`（puller 调度）
3. 在 `puller::pull_file` 入口加 trace 日志
4. 再跑 e2e_sync，看在哪一步链路断
5. 修复 + 移除 `#[ignore]` + 提交修复

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

按本文档现状：

| v0.X.Y | 必须包含 |
|--------|---------|
| **v0.2.5（patch）** | §2 puller 链路修复（提升到 P0） |
| **v0.3.0** | §1 ClusterConfig race + §4 TestNode 文档 + Linux 72h（§3） |
| **v0.4.0** | 跨版本互通自动化 + GUI / Web UI |

v0.3.0 路线图（`NEXT_STEPS_2026-05-13.md`）需要插入新 T2.6 = 修复 §2 + 取消 e2e_sync 的 `#[ignore]`。

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
