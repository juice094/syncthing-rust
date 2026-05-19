# Known Issues

> **维护原则**：发现的缺陷必须显式登记，避免误判项目成熟度。  
> **最后更新**：2026-05-16（v0.2.8 双节点真实网络测试通过 — Tailscale 双向文件同步验证）

本文档列举当前已知未修复的功能性 / 行为性问题。  
**这些问题决定了项目目前的"事实可用性"边界**。

---

## ⚠️ 项目阶段定位（2026-05-15 post-v0.2.6）

| 维度 | 状态 |
|------|------|
| 代码完成度 | ~85%（MVP 全部 module 编译；端到端链路通） |
| 单元测试覆盖 | 295/295 通过 |
| 连接层稳定性 | ✅ 12h+ 单机压测验证（T-F1 死锁已修复） |
| **端到端同步** | ✅ **已修复**（T2.6，见 §2） |
| 跨版本互通 | ✅ **已验证**（2026-05-14，Rust v0.2.6 ↔ Go v2.1.0，自动化脚本就绪） |
| 真实网络测试 | ✅ **已通过**（v0.2.8，Tailscale 双向 TLS+BEP+Index+Block Transfer+Watcher，见 §12） |
| 长跑（72h） | ⏳ 单机 12h 已验证；**双节点真实网络 72h** 为 v0.3.0 准入线 |
| 政企合规 | ❌ **未通过**（无国密、无 Prometheus、无审计日志、无 Transport 插件，见 §8） |
| 生产就绪度 | **alpha，已可用于研究 / 测试 / 个人私有部署，**不适用于政企生产**** |

类比：发动机 / 变速箱 / 车架 / 轮子都装好了，传动轴的卡扣插好了，可以踩油门跑。72h 单机耐力赛已拿到初赛成绩单，但真实路况（校园网 / 政企防火墙）刚刚发现需要加装四驱系统（Tailscale/Headscale 穿透）。

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

### §7.1 配置热重载死循环（P0，已造成事故）

**症状**：`notify` 事件风暴 → `daemon_runner.rs` 无 debounce 处理 → `info!` 日志高频输出 → 19 小时写出 21G → 磁盘 100% → 系统雪崩。

**根因**：
- `JsonConfigStore::watch()`（`syncthing-api/src/config.rs:203`）与 `daemon_runner.rs:448` 均未对文件系统事件做去抖；
- `notify` 在 overlayfs/云盘环境下可能产生虚假事件风暴；
- 热重载成功日志使用 `info!` 级别，落在高频路径。

**修复方向**：
- A. `daemon_runner` 增加 500ms~1s debounce（重置计时器模式）；
- B. `config.rs:271` / `daemon_runner.rs:457` 日志降为 `debug!`；
- C. `stream.next()` 对比 mtime/sha256，无变化跳过 reload。

**追踪**：v0.2.6 hotfix H-1

### §7.2 日志轮转仅按天、无单文件大小上限（P0，已造成事故）

**症状**：`tracing_appender::rolling::Rotation::DAILY` 在同一天内允许无限膨胀。事故中 `daemon.2026-05-13.log` 单文件 21G。

**根因**：`main.rs:144-146` 只有按日轮转 + `max_log_files(7)`，缺少单文件尺寸上限或按小时分割。

**修复方向**：改为按大小轮转（100MB）或至少 `Rotation::HOURLY`。

**追踪**：v0.2.6 hotfix H-2

### §7.3 无界 Channel 内存泄漏风险（P1）

**症状**：对端高速发包 / 文件系统事件风暴 / 连接事件堆积时，内存无界增长，最终 OOM。

**涉及位置**（生产代码中 `unbounded_channel`）：
- `syncthing-net/src/connection/mod.rs:126-127` — BEP message/incoming
- `syncthing-net/src/manager/mod.rs:100` — ConnectionManager events
- `syncthing-net/src/derp/server.rs:214` / `client.rs:75` / `pipe.rs:141-142` — DERP 全链路
- `cmd/syncthing/src/tui/daemon_runner.rs:202` — BepSessionEvent
- `crates/syncthing-sync/src/watcher.rs:30` — notify Event

**修复方向**：全部改为有界 channel（如 1024），发送端用 `try_send`，满时丢弃或反压。

**追踪**：v0.2.6 hotfix H-3

### §7.4 丢弃的 Receiver + 无界发送 = 确定泄漏（P1）

**症状**：`event_tx` 被持有并发送，但 `_event_rx` 被立即丢弃。消息进入无消费者的无界 channel，永不释放。

**涉及位置**：
- `relay_listener.rs:125,144`
- `tcp_transport.rs:191,269`
- `bep_adapter.rs:145,243`

**修复方向**：移除无用 channel，改用 `tracing::error!` 或原子标志；若需消费则保留 receiver。

**追踪**：v0.2.6 hotfix H-4

### §7.5 生产代码中存在 panic 路径（P2）

**症状**：外部输入（网络帧、事件类型、数据库文件）不可信，但代码使用 `panic!` / `unreachable!`，导致整进程崩溃。

**涉及位置**：
- `syncthing-api/src/events.rs:368` — `panic!("Wrong event type")`
- `syncthing-db/src/block_cache/mod.rs:89` — `panic!("Failed to open metadata tree")`
- `syncthing-net/src/derp/server.rs:415` — 非预期帧类型 panic
- `syncthing-net/src/manager/registry.rs:68` — `unreachable!()`

**修复方向**：全部改为 `error!` + 返回 `Err` 或跳过处理。

**追踪**：v0.2.6 hotfix H-5

### §7.6 Interval Loop 缺少优雅终止（P2）

**症状**：部分 `loop { interval.tick().await }` 未 `select!` 绑定 shutdown，daemon 停止时依赖 Tokio task abort，可能延迟资源释放。

**涉及位置**：
- `daemon_runner.rs:427-438` — session cleanup
- `discovery_tasks.rs:84` — global discovery
- `relay_listener.rs:38,109` — relay listeners
- `syncthing-api/src/events.rs:202,222` — event bus

**修复方向**：参照 `folder_model` 的 `select! { _ = shutdown.changed() => break }` 模式改造。

**追踪**：v0.2.6 hotfix H-6

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

## 路线图影响

按本文档现状（2026-05-15 Error-Budget 审计后）：

| v0.X.Y | 必须包含 |
|--------|---------|
| **v0.2.5** | ✅ §2 已修复 — 已发布 |
| **v0.2.6（hotfix）** | ✅ §7 运行时安全缺陷（H-1~H-6）：debounce + 日志上限 + 有界 channel + panic 清除 + shutdown select — 已合并 |
| **v0.3.0** | ✅ §11 Scanner 默认排除元数据 — 已修复 / §1 ClusterConfig race + §4 TestNode 文档 + §8 Transport Plugin + §9 Windows 块传输修复（Bug-1/2/3）+ §10 配置 UX（C-UX-1~5）+ §12 双节点真实网络通过 + 双节点 72h + Prometheus metrics + T3.1/T3.4 |
| **v0.4.0** | 国密 TLS（SM2/SM3/SM4）+ 证书外部化 + SQLite WAL + 审计日志 + 跨版本互通自动化 |

v0.3.0 路线图详见 [`NEXT_STEPS_2026-05-15.md`](./plans/NEXT_STEPS_2026-05-15.md)。

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
