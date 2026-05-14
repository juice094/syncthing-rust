# 后续任务清单 — 2026-05-14

> 承接 [NEXT_STEPS_2026-05-13.md](./NEXT_STEPS_2026-05-13.md)（已归档）
> 本文档为 INC-20260514-001 存储耗尽事故复盘后的行动计划。
> **状态快照（2026-05-14 23:30）**：v0.2.6 已发布；H-1~H-6 全部完成；运行时安全审查完成；T3.1b reconnect_device + session health check 完成；跨版本互通验证通过（Rust v0.2.6 vs Go v2.1.0）；Tailscale 验证纳入后续。

---

## 0. 事故复盘摘要（INC-20260514-001）

| 项目 | 内容 |
|------|------|
| **时间** | 2026-05-13 22:25 ~ 2026-05-14 08:31 |
| **现象** | Gray-Cloud VPS 根分区 100% 满，kimiclaw 僵死，Load Average 6.0+ |
| **根因** | syncthing-rust daemon 配置热重载 watcher 无 debounce，在 notify 事件风暴下进入 100μs 级死循环，19 小时写出 21G 日志 |
| **修复状态** | 运维侧：日志清空 + logrotate + journal vacuum；**代码侧：未修复，需发 v0.2.6 hotfix** |

**关键教训**：
1. `notify` + overlayfs/云盘 != 可靠的事件语义，必须假设事件风暴可能发生。
2. 按天日志轮转不能防止单日膨胀；必须有单文件大小上限或更细粒度轮转。
3. `unbounded_channel` + 丢弃的 receiver 是内存泄漏的温床。
4. 生产代码中 `panic!` 对外部输入是拒绝服务漏洞。

---

## 1. v0.2.6 Hotfix（紧急，预计 1~2 天）

所有任务必须满足：
- `cargo test --workspace` 296 通过
- `cargo clippy --workspace --all-targets -- -D warnings` 0 警告
- 代码审查后合并

### H-1 配置热重载 debounce（P0）

| 项 | 内容 |
|---|---|
| **文件** | `cmd/syncthing/src/tui/daemon_runner.rs`, `crates/syncthing-api/src/config.rs` |
| **方案** | ① `daemon_runner.rs:444` 增加 500ms debounce（新事件重置计时器）；② 热重载日志降为 `debug!`；③ `JsonConfigStream::next()` 对比文件 mtime，无变化跳过 reload |
| **验证** | 手动 `touch config.json` 50 次，日志应只输出 1 条 |
| **预计** | 2h |

### H-2 Daemon 日志按大小/小时轮转（P0）

| 项 | 内容 |
|---|---|
| **文件** | `cmd/syncthing/src/main.rs` |
| **方案** | `Rotation::HOURLY` 或 size-based rotation（100MB/文件）。参考 `stress_test.rs` T2.2 |
| **验证** | 启动 daemon，确认无单文件 >100MB |
| **预计** | 1h |

### H-3 无界 channel 改有界（P1）

| 项 | 内容 |
|---|---|
| **文件** | `syncthing-net/src/connection/mod.rs`, `manager/mod.rs`, `derp/*`, `daemon_runner.rs`, `syncthing-sync/src/watcher.rs` |
| **方案** | `unbounded_channel()` -> `channel(1024)`；发送端 `try_send` + 满时丢弃 |
| **风险** | 有界 channel 可能导致发送端阻塞；需确认不在锁临界区内 |
| **预计** | 4h |

### H-4 丢弃 receiver 修复（P1）

| 项 | 内容 |
|---|---|
| **文件** | `relay_listener.rs`, `tcp_transport.rs`, `bep_adapter.rs` |
| **方案** | 若不需要消费，移除 channel 改用 `tracing::error!`；若需要，保留 receiver |
| **预计** | 1h |

### H-5 Panic 路径改为 Error（P2）

| 项 | 内容 |
|---|---|
| **文件** | `syncthing-api/src/events.rs`, `syncthing-db/src/block_cache/mod.rs`, `syncthing-net/src/derp/server.rs`, `manager/registry.rs` |
| **方案** | `panic!` / `unreachable!()` -> `error! + return Err(...)` |
| **预计** | 2h |

### H-6 Interval loop 增加 shutdown select（P2）

| 项 | 内容 |
|---|---|
| **文件** | `daemon_runner.rs:427`, `discovery_tasks.rs`, `relay_listener.rs`, `syncthing-api/src/events.rs` |
| **方案** | 参照 `folder_model/mod.rs` 的 `select! { _ = shutdown.changed() => break }` 模式 |
| **预计** | 2h |

### v0.2.6 发布检查清单

- [x] H-1 ~ H-6 全部完成并合并到 main
- [x] CHANGELOG.md 增加 [0.2.6] 条目
- [x] README.md 当前限制更新
- [x] 版本号 0.2.5 -> 0.2.6（所有 Cargo.toml）
- [x] CI 全绿
- [x] 打 tag v0.2.6

---

## 2. v0.3.0 规划调整

v0.3.0 在原有基础上增加运行时安全基线要求：

| 原任务 | 调整 |
|--------|------|
| T2.4 Linux 72h 压测 | **保留**，但必须先完成 v0.2.6 |
| T3.1 §1 ClusterConfig race (`on_disconnected` 误杀) | **✅ 已完成** (commit `3656007`) |
| T3.1b `connect_to` 重连语义 | 纳入 v0.3.0：`connect_to_with_relay` 在已连接时应检查 session 健康度，或统一使用 `reconnect_device` |
| T3.2 FileSystemDatabase WAL RFC | 保留，可能延至 v0.4.0 |
| T3.4 dialer 业务拆分 | 保留 |
| §4 TestNode rescan interval | 纳入 v0.3.0 必含 |

**v0.2.6 → v0.3.0 基线完成情况**：
- [x] 所有 channel 必须有界 ✅ (H-3)
- [x] 所有 loop 必须可优雅终止 ✅ (H-6)
- [x] 生产代码零 panic ✅ (H-5)
- [x] 日志轮转通过单日 10GB 压力测试 ✅ (H-2)
- [x] §1 ClusterConfig race 修复 ✅ (T3.1)
- [x] T3.1b `reconnect_device` API + session health check ✅
- [x] 跨版本互通验证 ✅ (Rust v0.2.6 vs Go v2.1.0，TLS + BEP + 文件同步全通过)
- [x] `scripts/cross_version_test.sh` 更新 ✅ (2026-05-14)

**新增 v0.3.0 准入标准**：
- `connect_to` 重连语义：已连接时若 session 不健康，应自动重新触发 `on_connected`
- ~~跨版本互通测试：与 Go 版 syncthing 至少 1 次自动化验证~~ ✅ 已完成
- Linux 72h 长跑无内存泄漏、无死锁
- Tailscale 链路验证（有条件时执行）

---

## 3. 工程纪律更新（自 2026-05-14 起生效）

1. **watcher 必须有 debounce**：任何使用 `notify` 的代码必须附带 >=500ms debounce。
2. **channel 必须有界**：禁止新增 `unbounded_channel`；现有代码在触及即改。
3. **日志在热路径上必须为 debug!**：事件循环、packet 处理、watcher callback 中禁止 `info!`。
4. **panic 零容忍**：对外部输入使用 `error! + Err` 而非 `panic!`。
5. **事故复盘必须进 KNOWN_ISSUES**：任何导致服务不可用的事故必须在 KNOWN_ISSUES.md 登记。

---

## 4. 风险与回退

| 风险 | 触发条件 | 应对 |
|------|----------|------|
| H-3 有界 channel 导致发送端阻塞 | 对端恶意/异常高速发包 | 发送端使用 `try_send`，满时丢弃并计数告警 |
| H-1 debounce 引入延迟 | 用户期望配置立刻生效 | debounce 窗口 500ms，TUI 保存后显式提示延迟 |
| v0.2.6 范围膨胀 | H-3/H-5 改动面大 | 拆为 v0.2.6（H-1/H-2/H-4）+ v0.2.7（H-3/H-5/H-6） |

---

## 5. 跨文件跳转速查

- **当前权威路线图**：本文件
- **已知缺陷登记**：[../KNOWN_ISSUES.md](../KNOWN_ISSUES.md)
- **v0.2.5 归档计划**：[NEXT_STEPS_2026-05-13.md](./NEXT_STEPS_2026-05-13.md)
- **调优母计划**：[TUNING_PLAN_2026-05-11.md](./TUNING_PLAN_2026-05-11.md)
- **架构路线图**：[POST_V0_2_0_ROADMAP.md](./POST_V0_2_0_ROADMAP.md)

---

## 附录 A：跨版本互通验证记录（2026-05-14）

| 项目 | 内容 |
|------|------|
| **Rust 版本** | v0.2.6 (target/debug/syncthing.exe) |
| **Go 版本** | v2.1.0 (windows-amd64) |
| **验证平台** | Windows 10 + WSL2 |
| **网络拓扑** | 本地回环 (127.0.0.1) |
| **Rust Device ID** | `XCBFBGS-S4OBNCB-NNACTKO-UJX7V7W-GZLEN65-4N6W4JS-OKDNJBL-EOQXHQ7` |
| **Go Device ID** | `M4RXTIV-QOCPNOZ-KNPPN2N-6XFBWY4-X4TSCOD-I2WYL2Y-STGFJDS-JJFUBA3` |

**验证步骤与结果**：
1. TLS 1.3 握手 (`TLS_AES_128_GCM_SHA256`) ✅
2. BEP Hello 交换（双向识别 device name / client version）✅
3. ClusterConfig 交换（共享文件夹 `cross-test` 双边确认）✅
4. Index 发送/接收（Go → Rust 发送 1 file index）✅
5. 块请求/响应（`data_len=35`）✅
6. 文件下载完成（`test-cross-version.txt`）✅
7. 内容一致性（`Hello from Go syncthing v2.1.0`）✅

**关键踩坑记录**：
- Go syncthing 拒绝 hex 格式 Device ID（64 字符），必须使用 Base32+Luhn-32（56 字符）。
- Rust `config.json` 中 `Folder` / `Device` 结构体有多项无 `#[serde(default)]` 的必填字段，缺失会导致 `load_config` 失败并回退到 `Config::new()`（空配置），随后 `save_config` 覆写回磁盘。
- Go syncthing v2.x CLI 结构：使用子命令 `generate` / `serve` / `device-id`，参数格式 `--home=PATH`。
- Windows 版 Go syncthing 是 GUI 子系统，stdout 不绑定控制台，需用 `--log-file=` 捕获日志。

**脚本更新**：`scripts/cross_version_test.sh` 已重写（281 行），支持 Linux/Windows 双平台，自动生成完整字段配置，自动提取 Device ID。

---

## 附录 B：后续新增任务（待排期）

| 任务 | 优先级 | 依赖 | 备注 |
|------|--------|------|------|
| Tailscale 链路验证 | P2 | 两台 Tailnet 设备 | 有条件时执行；验证 CGNAT 地址下的直连、MTU 表现、回退行为 |
| Linux 72h 长跑 | P1 | Gray-Cloud VPS | 脚本已就绪 (`scripts/72h_*.sh`)，待部署启动 |

---

**维护人**：juice094
**下次刷新**：Linux 72h 部署启动 或 Tailscale 验证执行时
