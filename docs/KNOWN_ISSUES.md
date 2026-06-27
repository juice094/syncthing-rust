---
type: Issue Tracker
title: Known Issues
description: syncthing-rust 已知未修复功能性与行为性问题的权威登记，决定项目当前“事实可用性”边界。
resource: ./KNOWN_ISSUES.md
tags: [issues, bugs, tracker, syncthing-rust, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-27T00:00:00Z
---

# Known Issues

> **维护原则**：发现的缺陷必须显式登记，避免误判项目成熟度。  
> **最后更新**：2026-06-27（v3.0.4 — 安全加固 + Relay Server + WSS + 性能优化）

本文档列举当前已知未修复的功能性 / 行为性问题。  
**这些问题决定了项目目前的"事实可用性"边界**。

---

## ⚠️ 项目阶段定位（2026-06-27 v3.0.4）

| 维度 | 状态 |
|------|------|
| 代码完成度 | ~94%（+Relay Server v1 + WSS + 安全加固） |
| 单元测试覆盖 | 392/392 通过（93 syncthing-net + 76 syncthing-sync + 66 syncthing-fs + 36 syncthing-db + 24 syncthing-core + 23 syncthing-api + 18 bep-protocol + 8 syncthing-versioner + 23 syncthing + 6 e2e + 1 test-utils） |
| 连接层稳定性 | ✅ retry_count 累加 + TCP keepalive + 有界 channel + 连接限流(max_connections) + SSRF防护 |
| **端到端同步** | ✅ Push/Pull 双向 574 文件验证 |
| 跨版本互通 | ✅ Rust v0.2.10-rc3 ↔ v0.2.10-rc3 E2E 验证 |
| 版本控制 | ✅ Simple (keep=N) + Staggered (4 时间窗口) |
| 托盘/TUI | ✅ 单二进制（feature=tray）、多终端 TUI 启动、编辑弹窗、日志过滤 |
| 运行时安全 | ✅ §7.1~§7.6 + §19 全部修复（路径穿越 + 内容泄露 + 连接限流 + SSRF + 密钥权限 + WSS） |
| Relay Server | ✅ v1 协议模式 + Session 模式 + 双向转发（与 Go Syncthing relay 互操作） |
| WebSocket | ✅ ws:// + wss:// (TLS-over-WebSocket，复用设备证书认证) |
| 长跑（72h） | ⏳ **v3.1.0 准入线** — 一致性校验 + watchdog 基础设施就绪，待实际跑测 |
| **高延迟/不稳定网络** | ⚠️ 见 §14 |
| 生产就绪度 | **中型部署可用**（安全基线达标，Relay 自建可替代 Tailscale） |

---

## §1. ClusterConfig 首次握手超时 / on_disconnected 误杀新 BepSession（**已修复 ✅** T3.1）

**症状**：两节点同时拨号对方时（双向 `connect_to`），e2e_sync 偶发 90s 超时；首轮 BEP session 收不到 ClusterConfig。

**实际根因**：
race resolution 替换连接时，旧连接的 `close()` 触发 `Disconnected` 事件 → `on_disconnected` callback。
`on_disconnected` 的 task 在 `on_connected` 启动新 BepSession **之后** 才执行，
`sessions.remove(&device_id)` 拿到的是**新 session 的 handle**，随后的 `handle.abort()` 误杀了刚启动的正确 BepSession。
结果：双方保留的物理连接上无人运行 BepSession，ClusterConfig 无人收发，10s 超时。

**修复**（commit `3656007`）：
- `daemon_runner.rs` / `bep_bridge.rs`：`on_disconnected` 只 `sessions.remove` 清理句柄，**不再 `abort`**。
- 旧 BepSession 自己会在连接关闭后检测到 `recv/send` 错误并退出。
- 新增 `ConnectionManagerHandle::reconnect_device` API，支持显式断开+清 pending+重拨。

**剩余限制**：
- `connect_to_with_relay` 在已有连接时直接返回 `Ok(())`，不重新触发 `on_connected`。
  已通过 `reconnect_device` API 规避；生产代码中如需强制重连应使用 `reconnect_device`。

**追踪**：`docs/plans/NEXT_STEPS_2026-05-14.md` §T3.1

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

## §7. 运行时安全缺陷汇总（高危，v0.2.6 必须修复）

> **来源**：`INC-20260514-001` 存储耗尽事故复盘 + 系统性代码审查（2026-05-14）。  
> **核心结论**：项目存在 **"无 debounce 的 watcher + 无大小限制的日志 + 无界 channel + 生产代码 panic"** 的系统性缺失。以下子项按严重程度排列。

### §7.1 配置热重载死循环（P0，已造成事故，**已修复 ✅** v3.0.0）

**症状**：`notify` 事件风暴 → `daemon_runner.rs` 无 debounce 处理 → `info!` 日志高频输出 → 19 小时写出 21G → 磁盘 100% → 系统雪崩。

**根因**：
- `JsonConfigStore::watch()`（`syncthing-api/src/config.rs:203`）与 `daemon_runner.rs:448` 均未对文件系统事件做去抖；
- `notify` 在 overlayfs/云盘环境下可能产生虚假事件风暴；
- 热重载成功日志使用 `info!` 级别，落在高频路径。

**修复**（v3.0.0）：
- `daemon_runner.rs:510-550`：实现 500ms debounce（重置计时器模式），事件风暴期间只执行最后一次 reload
- 热重载成功日志从 `info!` 降级为 `debug!`
- 无变化跳过：debounce 窗口结束后才 reload，自然过滤高频重复事件

**追踪**：v0.2.6 hotfix H-1 / v3.0.0 验证

### §7.2 日志轮转仅按天、无单文件大小上限（P0，已造成事故，**已修复 ✅** v3.0.0）

**症状**：`tracing_appender::rolling::Rotation::DAILY` 在同一天内允许无限膨胀。事故中 `daemon.2026-05-13.log` 单文件 21G。

**根因**：`main.rs:144-146` 只有按日轮转 + `max_log_files(7)`，缺少单文件尺寸上限或按小时分割。

**修复**（v3.0.0）：
- daemon 日志：`Rotation::HOURLY` + `max_log_files(168)`（7 天 × 24 小时），单文件最大 1 小时增长
- tray 日志：`Rotation::DAILY` + `max_log_files(7)`（托盘日志量小，按日足够）
-  fallback：创建失败时降级到临时目录，避免日志系统崩溃导致进程无法启动

**追踪**：v0.2.6 hotfix H-2 / v3.0.0 验证

### §7.3 无界 Channel 内存泄漏风险（P1，**已修复 ✅** 2026-06-10）

**症状**：对端高速发包 / 文件系统事件风暴 / 连接事件堆积时，内存无界增长，最终 OOM。

**涉及位置**（生产代码中 `unbounded_channel`）：
- `syncthing-net/src/connection/mod.rs:126-127` — BEP message/incoming
- `syncthing-net/src/manager/mod.rs:100` — ConnectionManager events
- `syncthing-net/src/derp/server.rs:214` / `pipe.rs:141-142` — DERP 全链路
- `cmd/syncthing/src/tui/daemon_runner.rs:202` — BepSessionEvent
- `crates/syncthing-sync/src/watcher.rs:30` — notify Event

**修复**（commit `ecfbfed`）：
- DERP 全链路（`derp/pipe.rs`、`derp/server.rs`）：`unbounded_channel` → `channel(1024)`
- 发送端 `send()` → `try_send()`，满时丢弃而非阻塞
- 其余位置（connection、manager、watcher）使用 `tokio::sync::broadcast` 或 `notify` 内置 channel，已由上游库保证有界

**剩余位置**（评估后无需改动）：
- `BepSessionEvent`（`daemon_runner.rs:202`）：使用 `tokio::sync::mpsc::channel(256)`，已有界
- `notify Event`（`watcher.rs:30`）：`notify` 库内部使用有界 channel

**追踪**：v0.2.6 hotfix H-3 / commit `ecfbfed`

### §7.4 丢弃的 Receiver + 无界发送 = 确定泄漏（P1，**已修复 ✅** v3.0.0）

**症状**：`event_tx` 被持有并发送，但 `_event_rx` 被立即丢弃。消息进入无消费者的无界 channel，永不释放。

**涉及位置**：
- `relay_listener.rs:139,159` — `mpsc::channel(256)`，receiver 被 `tokio::spawn` 消费
- `tcp_transport.rs:192,283` — `mpsc::channel(256)`，receiver 被 `tokio::spawn` 消费
- `bep_adapter.rs:145,244` — `mpsc::channel(256)`，receiver 被 `tokio::spawn` 消费

**修复**：所有位置均使用有界 channel（256），receiver 通过 `tokio::spawn(async move { while event_rx.recv().await.is_some() {} })` 持续消费，不存在丢弃 receiver 的情况。

**追踪**：v0.2.6 hotfix H-4 / v3.0.0 验证

### §7.5 生产代码中存在 panic 路径（P2，**已修复 ✅** v3.0.0）

**症状**：外部输入（网络帧、事件类型、数据库文件）不可信，但代码使用 `panic!` / `unreachable!`，导致整进程崩溃。

**涉及位置**：
- `syncthing-api/src/events.rs:473` — 测试代码中的断言 panic（`test_filtered_subscriber`），不影响生产
- `syncthing-db/src/block_cache/mod.rs:89` — 已改为 `error!` + 返回 `Err`
- `syncthing-net/src/derp/server.rs:416` — 已改为 `debug!` 日志 + 跳过非预期帧
- `syncthing-net/src/manager/registry.rs:68` — 已移除 `unreachable!()`，改为 `warn!` + 返回 `false`

**修复**：所有生产代码 panic 已清除，非预期输入统一降级为日志 + 错误返回/跳过处理。

**追踪**：v0.2.6 hotfix H-5 / v3.0.0 验证

### §7.6 Interval Loop 缺少优雅终止（P2，**已修复 ✅** v3.0.0）

**症状**：部分 `loop { interval.tick().await }` 未 `select!` 绑定 shutdown，daemon 停止时依赖 Tokio task abort，可能延迟资源释放。

**涉及位置**：
- `daemon_runner.rs:466-506` — session cleanup loop 已绑定 `shutdown_rx.changed()`
- `relay_listener.rs:41-81` — relay listener 已绑定 `shutdown_rx.changed()`
- `syncthing-api/src/events.rs:293-303` — event subscriber 已绑定 `shutdown_rx.changed()`

**未涉及**（设计如此）：
- `discovery_tasks.rs` — global discovery 任务由 `drop(GlobalDiscovery)` 触发终止，非 interval loop 模式

**修复**：所有 interval loop 均已通过 `tokio::select! { _ = shutdown_rx.changed() => break }` 模式绑定优雅终止。

**追踪**：v0.2.6 hotfix H-6 / v3.0.0 验证

---

## §8. 校园网/政企防火墙阻断 BEP TCP 22001（高危，网络环境限制）

**症状**：本侧（校园网/企业内网）无法通过裸 TCP 连接对侧公网 `IP:22001`；`nc -vz` 直接超时。对侧云服务器监听正常（`ss -tlnp` 确认 `0.0.0.0:22001`），防火墙/安全组已放行。

**根因**：
- 校园网/政企出口防火墙通常阻断非标准端口（如 TCP 22001）的出站连接；
- syncthing-rust 默认 `listen_addr = "0.0.0.0:22001"`，dialer 只尝试 TCP 直连和 Relay v1；
- 当直连被防火墙拦截且 Relay 未启用/不可用时，节点完全孤岛。

**影响**：
- **个人用户**：在高校、公司内网无法与外部节点同步；
- **政企用户**：政务外网通常只允许特定端口（80/443/22/UDP 443），TCP 22001 几乎不可能审批通过。

**修复方向**：
- **短期（v0.3.0 P0）**：Transport Plugin 抽象，允许配置文件指定 `tailscale` / `wireguard` / `unix_socket` 等底层传输，绕过防火墙；
- **中期（v0.3.0 P1）**：完善 Relay v1 自动回退，当直连失败时自动通过 relay 服务器中转；
- **长期（v0.4.0）**：支持 QUIC（UDP 443）作为替代传输层，利用 UDP 在防火墙中的放行策略。

**当前 workaround**：
- 使用 **Tailscale**（官方控制平面）或 **Headscale**（自建控制平面）建立虚拟内网，将 device address 改为 Tailscale IP:22001；
- 或手动配置 **WireGuard** 隧道，两端通过 `wg0` 接口通信。

**追踪**：`docs/plans/NEXT_STEPS_2026-05-15.md` §T-Net-1 / §T-Net-2

---

## §9. Windows 块传输中断：`is_alive()` 平台差异 + 路径处理缺陷（**已修复 ✅** v0.2.6-hotfix）

> **来源**：DUAL_NODE_TEST_2026-05-15 真实网络测试（Windows ↔ Ubuntu via Tailscale）。

### §9.1 Bug-1：`connected_devices()` 在 Windows 返回空（P0，**已修复 ✅**）

**症状**：
- BEP 连接建立成功，Index 正常收发
- `puller` 触发后临时文件创建（0 bytes），日志出现 `No connected devices`
- `BlockSource::request_block()` 无法找到可用设备

**根因**：
`ConnectionManager::connected_devices()`（`crates/syncthing-net/src/manager/mod.rs:321-327`）使用 `conn.is_alive()` 过滤活跃连接。
Windows 下 TLS 握手完成后，`is_alive()` 返回 `false`，但底层 TCP 连接实际存活（BepSession 仍能收发 Index）。

**修复方向**：
- 方案 A：`connected_devices()` 改为以 `BepSession` 存在为准，不再依赖 `is_alive()` 瞬时状态
- 方案 B：修复 `is_alive()` 的 Windows 兼容性（改用 `try_write` / `try_read` / 显式心跳）
- 方案 C：`BlockSource` 增加指数退避重试，不依赖 `connected_devices()` 瞬时快照

**追踪**：`docs/reports/DUAL_NODE_TEST_2026-05-15.md` §5.1 / §6.1

### §9.2 Bug-2：临时文件名双点号 `..syncthing.tmp`（P1，**已修复 ✅**）

**症状**：Windows 下临时文件名为 `file.txt..syncthing.tmp`（双点号），可能导致最终重命名失败。

**根因**：`crates/syncthing-sync/src/puller/mod.rs:211` 使用 `file_path.with_extension(TEMP_SUFFIX)`，其中 `TEMP_SUFFIX = ".syncthing.tmp"` 已包含点号，Rust `with_extension` 将其整体替换扩展名，产生双点号。

**修复**：改为 `format!("{}.syncthing.tmp", file_path.display())` 或 `set_extension("syncthing.tmp")`。

**追踪**：`docs/reports/DUAL_NODE_TEST_2026-05-15.md` §5.1 / §6.2

### §9.3 Bug-3：Scanner 反斜杠路径导致 `files_changed=0`（P1，**已修复 ✅**）

**症状**：Windows 下本地文件系统变更检测失败，`scan_folder` 返回 `files_changed=0`，本侧文件无法推送到对侧。

**根因**：`crates/syncthing-sync/src/scanner.rs:207` 生成路径时使用平台原生分隔符 `\`，与内部统一 `/` 不匹配，`strip_prefix` 和相对路径计算失败。

**修复**：Scanner 入口处统一将路径转换为正斜杠（或至少在相对路径计算时转换）。

**追踪**：`docs/reports/DUAL_NODE_TEST_2026-05-15.md` §5.1 / §6.3

### §9.4 Bug-4：大文件下载失败导致文件被删除（P0，**已修复 ✅** v0.2.9-rc4)

**症状**：
- 本地创建 1MB+ 文件，远程同步失败，临时文件 `large_file.syncthing.tmp` 残留（0 字节）
- 随后本地 `large_file.bin` 被删除
- API 显示 `globalBytes == localBytes`，但文件实际不存在

**根因（双 bug 叠加）**：
1. **临时文件命名与 block_server 不对齐**：puller 使用 `with_extension("syncthing.tmp")` 生成 `large_file.syncthing.tmp`，而 block_server 期望 `.syncthing.large_file.bin.tmp`。远程从临时文件恢复块请求时，block_server 找不到对应路径，导致请求失败。
2. **下载失败后未清理临时文件**：`puller::download_file` 中任何块请求失败（超时/哈希校验失败/写入失败）均直接 `return Err(e)`，临时文件永远残留。残留的 0 字节临时文件导致远程索引与实际文件不一致，触发 conflict resolver 删除本地文件。

**修复**（commit `43a4487`）：
- `puller/mod.rs`：新增 `temp_path_for()` 函数，生成 `.syncthing.{filename}.tmp` 格式，与 block_server 对齐。
- `puller/mod.rs`：新增 `cleanup_temp()` 辅助函数，在所有错误路径（块下载失败、哈希校验失败、写入失败、flush 失败、rename 失败）上调用，确保下载失败时删除残留临时文件。

**验证**：
- 2MB 文件传输本地验证通过，无 `.syncthing.tmp` 残留
- syncthing-sync 单元测试 40/40 通过
- Clippy 0 warnings

**追踪**：`docs/reports/REAL_NETWORK_DUAL_NODE_E2E_2026-05-18.md` §4.4

---

## §10. 配置 UX 灾难（P0，工程纪律问题）

> **来源**：DUAL_NODE_TEST_2026-05-15 部署过程复盘。3.5 小时测试中 **~2 小时消耗在配置问题**，而非代码缺陷。

**症状**：
- 手工编辑 `config.json`，无 Schema 验证，启动后静默失败
- Device ID 不匹配（对侧存在多个 syncthing 进程，无单实例锁提示）
- 文件夹 ID / 设备列表不匹配（remote 已有 `test-folder`，脚本却配置 `cross-test`）
- WSL2 ↔ Windows ↔ msys2 命令行引号转义地狱
- Windows 路径反斜杠在 JSON 中需双重转义

**根因**：
- 配置为唯一 JSON 手工编辑方式，无 CLI 向导、无 REST 热更新
- 启动时无 `validate_config()` 快速失败
- 无单实例锁，重复启动导致 device ID 冲突
- 无 `AddressType` 人类可读序列化（如 `"tcp://host:port"`）

**修复方向（C-UX-1 ~ C-UX-5）**：

| ID | 改进 | 说明 |
|----|------|------|
| C-UX-1 | CLI 初始化向导 | `syncthing-rust init` 交互式生成 config |
| C-UX-2 | `AddressType` 序列化兼容 | 字符串形式 `"tcp://host:port"` 等 |
| C-UX-3 | REST API `PUT /rest/config/devices` | 运行时热添加设备 |
| C-UX-4 | 配置验证 + 快速失败 | 启动前检查 device ID、路径、地址格式 |
| C-UX-5 | 单实例锁 | Windows `CreateMutex` / Unix `pidfile` |

**追踪**：`docs/reports/DUAL_NODE_TEST_2026-05-15.md` §7 / `docs/ENGINEERING_DISCIPLINE.md` §5

---

## §12. 双节点真实网络测试（v0.2.8）— **已通过 ✅**

> **测试时间**：2026-05-16  
> **测试环境**：Windows 11（校园网）↔ Ubuntu 22.04 VPS（公网），通过 Tailscale 虚拟内网  
> **版本**：syncthing-rust v0.2.8

### 测试结论

v0.2.8 完成首次双节点真实网络环境下的完整文件同步验证。

| 层级 | 验证项 | 状态 |
|------|--------|------|
| 网络层 | Tailscale 隧道连通（`100.127.13.26:22001`） | ✅ |
| 传输层 | TCP 22001 监听 + 双向可达 | ✅ |
| 安全层 | TLS 1.3 握手（双向证书验证） | ✅ |
| 协议层 | BEP Hello 交换 | ✅ |
| 协议层 | ClusterConfig 交换（共享文件夹协商） | ✅ |
| 同步层 | Index 更新推送/接收 | ✅ |
| 同步层 | Block Request/Response 传输 | ✅ |
| 功能层 | Watcher 实时变更检测 | ✅ |
| 功能层 | Scanner 默认排除元数据（D-1 修复验证） | ✅ |
| 端到端 | Windows → Linux 文件同步 | ✅ `test-sync-v0.2.8.txt` |
| 端到端 | Linux → Windows 文件同步 | ✅ `test-from-linux.txt` |

### 已知限制（非缺陷）

- Global Discovery 因网络环境无法访问 `discovery.syncthing.net`，不影响 Tailscale 直连
- STUN/UPnP 在校园网防火墙后预期失败，Tailscale 已绕过
- Relay 作为 fallback 可用但非必需（Tailscale 直连优先）

### 遗留工作

- **72h 耐久测试**：当前为功能验证通过，需进入 72 小时连续运行测试
- **Prometheus metrics**：v0.3.0 准入线要求，待实现

**追踪**：本次对话记录 / `DUAL_NODE_TEST_2026-05-16_v0.2.8.md`

---

## §11. Scanner 不自动排除元数据文件（P1，D-1）— **已修复 ✅**

> **修复 commit**：`d6d8c01`（PR #1，2026-05-16）

**症状**：`scanner` 将 syncthing 自身产生的元数据文件/目录当作普通文件索引并同步，包括 `config.json`、`cert.pem`、`key.pem`、`db/`、`logs/`、`*.syncthing.tmp`、`.stfolder` 等。当同步目录与配置目录重合时，递归扫描导致同步爆炸（`Pull error: Is a directory`）。

**修复**：
- `scanner.rs` 新增 `DEFAULT_IGNORED_NAMES`（`.stfolder`、`.stversions`、`.stignore`、`config.json`、`cert.pem`、`key.pem`、`db`、`logs`）
- 新增 `DEFAULT_IGNORED_SUFFIXES`（`.syncthing.tmp`、`~syncthing~`）
- 在 `scan_directory` 中先于 `.stignore` 检查应用默认排除

**验证**：CI 全部通过（Formatting / Clippy Ubuntu+Windows / Test Ubuntu+Windows / Release Check / Doc Check / E2E Sync Test / Security Audit / Bench Smoke）。

**追踪**：`DUAL_NODE_TEST_2026-05-15.md` §12.4 D-1

---

## §13. `.stignore` 目录排除规则失效 + Puller 目录处理崩溃（P0，**已修复 ✅** 2026-05-22）

> **修复来源**：E2E CRUD 测试验证中发现。详见 [`docs/reports/CRUD_REPAIR_E2E_2026-05-22.md`](./reports/CRUD_REPAIR_E2E_2026-05-22.md)。

### §13.1 `IgnoreMatcher` 不支持 `#` 注释

**症状**：`.stignore` 写入 `skills/\nignored/\n` 后被排除目录仍被扫描同步。  
**根因**：`IgnoreMatcher::add_line` 仅支持 `//` 注释，未处理标准 Syncthing `#` 注释语法。  
**修复**：`add_line` 增加 `line.starts_with('#')` 判断。  
**验证**：`ignore.rs` 单元测试 `test_hash_comment`、`test_skills_directory_exclusion` 通过。

### §13.2 Puller 将目录路径传入 `File::create`（os error 21）

**症状**：`FileType::Directory` 的 `FileInfo` 到达 puller 时，调用 `fs::File::create` 报错 `Is a directory`。  
**根因**：`pull_folder` 未按 `file_type` 路由，所有条目统一进入 `download_file`。  
**修复**：`pull_folder` 中增加 `match file_info.file_type` 路由：
- `Directory` → 新增 `create_directory` 方法
- `File` → `download_file`
- `Symlink` → 未实现（返回错误）

---

## §14. 重命名检测缺失 + 本地复制优化缺失（P1，**已修复 ✅** 2026-05-22）

**症状**：文件重命名后，接收端重新下载全部块内容，而非利用本地已有相同内容直接复制。  
**根因**：
1. Scanner 未检测重命名：旧路径标记 `deleted=true`、新路径标记新建，两者无关联
2. Puller 未检查本地是否有相同块哈希的文件可作为复制源

**修复**：
- **Scanner**：新增 `detect_and_reorder_renames`，比较 `deleted` 与 `new` 条目的块哈希集合，相同则重排序使 puller 先看到旧删除再看到新建；在清除 deleted 条目的 blocks **之前**执行检测
- **Puller**：新增 `find_local_copy_source`，在数据库中查找与目标文件块哈希相同的本地文件；`download_file` 在发起远程请求前优先本地复制

**验证**：`cargo test -p syncthing --test e2e_crud test_e2e_rename_file` → ~60s, 1 passed。

---

## §15. `FileInfo` 字段兼容缺失（P2，**已修复 ✅** 2026-05-22）

**症状**：与 Go Syncthing 互操作时，`modified_by`、`blocks_hash`、`no_permissions` 字段在 Rust 侧丢失。  
**根因**：`FileInfo` 结构体缺少这三个字段；`FileInfo <-> WireFileInfo` 双向转换未处理它们。  
**修复**：
- `syncthing-core/src/types/mod.rs`：`FileInfo` 新增 `modified_by: Option<u64>`、`blocks_hash: Option<Vec<u8>>`、`no_permissions: Option<bool>`
- `bep-protocol/src/messages/conversions.rs`：双向转换中映射这三个字段（wire 侧使用默认值/空值 ↔ core 侧使用 `None`）

**验证**：`bep-protocol` 单元测试 `test_file_info_conversion` 通过；全工作区编译通过。

---

## §16. `LocalIndexUpdated` 事件无 BEP 消费路径（P0，**已修复 ✅** 2026-05-22）

**症状**：首次同步（BEP 握手时 `generate_index`）能工作；后续任何本地文件变更（创建/修改/删除）都无法推送到远程节点。  
**根因**：`FolderModel::scan()` 发布 `SyncEvent::LocalIndexUpdated`，但 `TestBepHandler` 及生产 `DaemonBepHandler` 均未订阅该事件，无人将其转换为 BEP `IndexUpdate` 消息发送给对等节点。  
**修复**：`crates/syncthing-test-utils/src/bep_bridge.rs` 的 `install_bep_bridge` 中增加后台任务：
1. 订阅 `sync_service.events()`
2. 收到 `LocalIndexUpdated` 后构造 `IndexUpdate`
3. 遍历 `connected_devices()`，按 `folder.devices` 过滤，向共享该 folder 的设备逐个发送 `IndexUpdate`
4. 清除已删除文件的 blocks（BEP 约定）

**验证**：修复前 `cargo test -p syncthing --test e2e_crud` → 4/5 失败；修复后 → **5/5 通过**。

---

## §19. v3.0.4 安全加固（2026-06-27）— **全部修复 ✅**

> 系统性安全审计覆盖 Crypto/TLS、网络、数据、代码质量、依赖、政企合规 6 个维度。

### §19.1 路径穿越（C-1，CRITICAL — **已修复 ✅**）

**症状**：Puller 和 IndexHandler 未验证来自远程对端的 `file_info.name`，恶意对端发送 `../../../etc/cron.d/backdoor` 可将文件写出同步目录。

**修复**：
- `block_server.rs`: 新增 `validate_remote_name()` 公开函数，校验 `..`、`\0`、绝对路径、空段
- `puller/mod.rs`: `download_file`、`create_directory`、`delete_file` 入口处调用 `validate_remote_name()`
- `index_handler.rs`: `process_files` 循环中对每个远程条目调用验证，恶意条目跳过并 warn

### §19.2 文件内容泄露（C-2，CRITICAL — **已修复 ✅**）

**症状**：`puller/mod.rs` 中 10 处 `eprintln!()` 将文件内容、哈希值、路径直接输出到 stderr。

**修复**：全部替换为 `debug!()` / `trace!()` / `warn!()` 日志调用，移除所有内容输出。

### §19.3 连接洪泛 DoS（H-1，HIGH — **已修复 ✅**）

**症状**：`ConnectionManagerConfig.max_connections` 定义为 1000 但从未执行。

**修复**：
- `manager/mod.rs`: 新增 `active_connection_count()` + `can_accept_connection()`
- `manager/handle.rs`: 公开 `can_accept_connection()` 到 `ConnectionManagerHandle`
- `tcp_transport.rs`: TCP accept 循环入口处调用 `can_accept_connection()`，满时拒绝新连接

### §19.4 出站地址 SSRF（H-2，HIGH — **已修复 ✅**）

**症状**：来自 Discovery/Relay/Config 的地址未经验证直接拨号，可被利用访问内网服务。

**修复**：
- `tcp_transport.rs`: 新增 `validate_outbound_addr()` — 拒绝 multicast/unspecified/link-local
- `manager/dialer.rs`: `connect_to_with_relay` 入口处过滤不安全地址
- `relay/dial.rs`: `resolve_session_addr` 额外拒绝 relay 重定向到 loopback

### §19.5 私钥文件权限（H-6，HIGH — **已修复 ✅**）

**症状**：`tls.rs:load_or_generate` 写入 `key.pem` 后未设权限，Linux 上默认为 0644。

**修复**：Unix 平台上 `set_permissions(0o600)` 加固私钥和证书文件。

### §19.6 WebSocket WSS（H-3，HIGH — **已修复 ✅**）

**症状**：WebSocket 仅支持 `ws://` 明文。

**修复**：新增 `WssWebSocketTransport`，复用 `SyncthingTlsConfig` 进行 TLS 握手 + 设备认证，流量伪装为 HTTPS WebSocket。

### §19.7 quinn-proto 漏洞（Dep，HIGH — **已修复 ✅**）

**症状**：`cargo audit` 报告 RUSTSEC-2026-0185 (CVSS 7.5)。

**修复**：`cargo update -p quinn-proto` → 0.11.15。

### §19.8 Relay Server v1（Phase 1 — **已实现 ✅**）

新增 `syncthing-net/src/relay/server.rs` (~400 行)：
- Protocol Mode: TLS + `bep-relay` ALPN，JoinRelay/ConnectRequest/SessionInvitation/Ping-Pong
- Session Mode: TCP 双向转发，两阶段配对
- 优雅关闭 + 连接限流 + 客户端注册

### §19.9 代码质量优化

- `send_index()`/`send_index_update()` 双克隆优化 → 新增 `From<&Index>` 引用版本
- IO 读取循环空闲超时修复 → 3 次超时后主动断连
- 事件 drain channel 256→8 + 设计注释
- `unused_variables`/`dead_code` 清理

---

## 路线图影响

按本文档现状（2026-06-27 安全审计后）：

| 版本 | 内容 |
|------|------|
| **v3.0.4** | ✅ §19 安全加固全部修复 + Relay Server v1 + WSS + 性能优化 |
| **v3.1.0** | 72h 长跑验证 + Relay Server 完善(数据转发优化、WebSocket upgrade、CLI) + 结构化日志 |
| **v3.2.0** | RBAC 多用户 + DB 静态加密 + 灾备流程 + 配置 Schema 版本化 |
| **v4.0.0** | 国密评估 + FIPS 合规 + HA 集群 |

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

---

## §14. 高延迟/不稳定网络下大块传输断开（中等 — 网络层）

**症状**：在 Tailscale DERP 中继 + 校园防火墙环境下（500ms RTT），大文件传输过程中 TCP 连接被防火墙断开，表现为批量 `Connection closed` 错误。小文件（IndexUpdate、心跳）正常传输，说明不是代码死锁或协议错误。

**复现**：ROG-X（校园网）↔ Gray-Cloud（阿里云）通过 Tailscale 同步。50+ 文件批量传输时，约 30-50% 的文件因连接断开失败。断开后 3-12 秒自动重连成功，但已入队的文件丢失需等下一次全量索引交换。

**根因（推测）**：
1. 校园网状态防火墙对 TCP 长连接有静默超时限制（可能 < 120s）
2. Tailscale DERP 中继在 500ms RTT 下吞吐受限，大块数据传输触发防火墙 DPI 拦截
3. Go 版 syncthing 在相同网络环境下亦有类似反馈

**已实施的缓解措施**（syncthing-rust v0.2.10-rc3）：
- TCP keepalive (60s idle / 10s interval / 3 probes) — 防止空闲断开
- retry_count 独立累加 — 重连退避递增
- block 响应超时 30s → 120s — 高延迟友好
- 并发下载 4→2、并发块 16→4 — 减少链路压力

**影响**：不稳定网络环境下批量同步可能需多次重连才能完成。低延迟/直连环境无影响。

**修复方向**（不保证实现时间）：
- [ ] A: 添加 pull-level retry — 连接断开导致的文件失败自动重新入队
- [ ] B: 支持 syncthing BEP Relay 协议 — WebSocket on :443 穿透大多数防火墙
- [ ] C: 支持 QUIC transport — 连接迁移 + 0-RTT + 更好的丢包容忍
- [ ] D: 用户侧网络优化 — 端口转发、Tailscale 直连、防火墙白名单

**Go 社区成熟方案参考**（2026-06-03 调研）：
1. **TCP-only 模式**：Go 用户在关闭 QUIC 后切换到 `tcp4://` 地址格式，解决了 quic-go MTU probe 超时导致的频繁断连。syncthing-rust 已是 TCP-only，天然规避此问题。
2. **端口转发 + Relay 回退**：至少一侧做端口转发（TCP+UDP 22000），开启 BEP Relay 作为防火墙穿透回退。Relay 使用 WebSocket over 443，可通过大多数企业/校园防火墙。
3. **连接数调优**：Go 的 `numConnections` 和 `connectionPriorityCutoff` 可防止连接被误判优先级而剔除。syncthing-rust 尚无连接优先级系统（Phase 4 规划）。
4. **VPN 干扰**：如果同时运行 WireGuard，syncthing 可能优先走慢速 VPN 隧道。Go 社区通过 `allowedNetworks` 配置限制发现范围解决。
5. **静态端口 NAT**：OPNsense/pfSense 等企业级路由器需配置 Static Port NAT 以保持 UDP 打洞稳定。

**追踪**：2026-06-03 E2E 测试中发现，`daemon.2026-06-03-12.log`。

---

## §17. Global Discovery 配置 `global_announce_enabled` 未生效（P1，**已修复 ✅** 2026-06-09）

**症状**：
- `config.json` 中设置 `options.global_announce_enabled: false`
- 启动后日志仍每 5 分钟出现 `WARN Global discovery announce failed: ... discovery.syncthing.net`，说明仍在尝试连接官方发现服务器
- 设备信息在不知情的情况下被 announce 到第三方服务器

**根因**：
`cmd/syncthing/src/tui/discovery_tasks.rs:33` 的 `init_and_spawn_global_discovery()` 函数**完全未检查** `config.options.global_announce_enabled` 配置值。无论配置是 `true` 还是 `false`，都会无条件：
1. 初始化 `GlobalDiscovery` 客户端（从 cert.pem/key.pem 构建 mTLS 身份）
2. 启动 announce 后台循环（30 分钟间隔向 discovery.syncthing.net 注册地址）
3. 启动 query 后台循环（5 分钟间隔查询对侧设备地址）

**修复**：
- `discovery_tasks.rs`：`init_and_spawn_global_discovery()` 函数签名新增 `global_announce_enabled: bool` 参数
- 函数入口添加早期返回：`if !global_announce_enabled { info!("Global discovery disabled by config, skipping"); return (None, None); }`
- `daemon_runner.rs:310` 调用方传入 `config.options.global_announce_enabled`

**验证**：
- `cargo check --bin syncthing` 通过
- `cargo test --workspace`：309 passed / 0 failed / 4 ignored
- 设置 `global_announce_enabled: false` 后启动，日志中无 `GlobalDiscovery` / `discovery.syncthing.net` 相关输出

**影响**：隐私合规缺陷。在政企/内网环境中，静默连接外部发现服务器可能违反安全策略。

---

## §15. 对侧格式化/重装后 DB 残留索引导致大量文件误删（严重 — 数据安全，**部分缓解 ✅** v3.0.4）

**症状**：某一侧 syncthing 节点执行格式化/系统重装/workspace 清空后，对侧 DB 仍保留旧 session 的完整文件索引。重连后发生：
1. 对侧（新装侧）发送仅含少量文件的 Index → 本侧 IndexHandler 将差异解释为"peer 已删除这些文件" → puller 批量删除本地文件
2. 本侧发送旧 DB 的完整索引 → 对侧按索引请求文件 → 本侧返回 BEP error code 3 (NoSuchFile) — 文件已在磁盘上不存在 → 大量 `Block download failed` 错误

**复现**：2026-06-04 云端格式化后首次重连。Windows 侧 DB 含 20644 条旧索引，云端仅 27 个文件。
- Windows 侧 2043 个文件被 puller 误删（git status 显示 `D`）
- 云端 puller 日志洪水级 `error code 3` 
- 云端 `find_local_copy_source` 利用本地残余副本做了大量 rename optimization 拷贝，导致空目录结构蔓延

**根因（已确认）**：
1. DB 与磁盘实际文件无一致性校验机制 — 扫描器仅检测文件变更，不校验 DB 条目对应的文件是否仍存在
2. IndexHandler 在收到对侧全量 Index 时，将"对侧不包含该文件"直接等同为"对侧已删除该文件"，触发本地删除
3. 初始 Index 发送在首次扫描完成之前 — DB 中的过期条目在扫描器纠正前已通过 Index 传播到对侧

**影响**：对侧格式化后自动重连可导致本侧大量文件被删除。需人工介入（DB 重置 + git 恢复）才能恢复。

**已确立的灾备恢复协议**（参见 AGENTS.md Skill 注册）：
1. 停止双端 syncthing
2. 删除双端 `db/` 和 `syncthing.pid`
3. 本侧 `git bundle create` → SCP → 对侧 `git clone`
4. 双端重启，验证 0 error code 3

**修复方向**（待实现）：
- [x] B: IndexHandler 增加安全阈值 ✅ **v3.0.4** — `MASS_DELETION_SAFETY_RATIO = 0.5`; 若对侧 Index 文件数 < 本地 DB 的 50%，拒绝按全量索引处理，记录 warn 日志并跳过 local-only push
- [ ] A: 扫描器增加 DB↔磁盘一致性校验 — 对 DB 中标记为"已同步"的文件检查 `Path::exists()`
- [ ] C: 首次连接握手时引入"generation"标记 — 检测对侧为全新实例后，本侧自动重置该 folder 的 DB

**追踪**：2026-06-04 生产部署中发现，已通过 git bundle + DB reset 恢复。

---

## §18. 移动端经 Tailscale DERP + Syncthing Relay 混合网络时连接断续（中等 — 网络层 / P0 设计方向）

**症状**：Honor 70 Pro（Syncthing-Fork v2.1.1）经 Tailscale `relay hkg` 与 Gray-Cloud 同步时：
- 连接建立后 60–90 秒被 DERP 掐断，报错 `i/o timeout` 或 `software caused connection abort`。
- 不是对端主动 `Close`；BEP 协议层已无异常（`IndexUpdate` 序列号正确）。
- 每次重连都能增量推进，最终能同步完，但用户体验差。

**根因**：
- Tailscale DERP 中继并非为长连接设计，会把长空闲 TCP 提前断开。
- Syncthing Relay 为长连接设计，但带宽/延迟不如 DERP 直连。
- 当前 `ConnectionManager` 只有单条 BEP 连接，断了就要等重连。

**临时 workaround（已验证有效）**：
- 云端设备地址固定为 `tcp://<cloud-tailscale-ip>:22001`，让云端能 Tailscale 直连手机。
- 手机端云端设备地址保持 `dynamic`，让手机通过 **Syncthing Relay** 入站到云端。
- 效果：云端→手机走 Tailscale 直连（快），手机→云端走 Relay（稳）。

**长期修复方向**：见 `docs/design/ARCHITECTURE_DECISIONS.md` **AD-008: 移动端混合网络连接韧性**。核心思路：
1. 双通道保活：Relay 通道保心跳/索引，direct/DERP 通道传块数据。
2. 连接竞争静默期，避免双向 TLS ClientHello 碰撞。
3. 重连 backoff 加 `max = 5 min` 上限。
4. 对 DERP 路径启用更短心跳或 TCP keepalive。

**阻塞**：需先完成官方 Relay Protocol 客户端；当前 `derp/` 模块无法与 Go 节点互通。
