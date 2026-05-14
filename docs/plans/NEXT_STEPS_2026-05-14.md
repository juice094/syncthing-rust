# 后续任务清单 — 2026-05-14

> 承接 [NEXT_STEPS_2026-05-13.md](./NEXT_STEPS_2026-05-13.md)（已归档）
> 本文档为 INC-20260514-001 存储耗尽事故复盘后的行动计划。
> **状态快照（2026-05-14）**：v0.2.5 已发布；运行时安全审查完成；v0.2.6 hotfix 已立项。

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

- [ ] H-1 ~ H-6 全部完成并合并到 main
- [ ] CHANGELOG.md 增加 [0.2.6] 条目
- [ ] README.md 当前限制更新
- [ ] 版本号 0.2.5 -> 0.2.6（所有 Cargo.toml）
- [ ] CI 全绿
- [ ] 打 tag v0.2.6

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

**新增 v0.3.0 准入标准**：
- `connect_to` 重连语义：已连接时若 session 不健康，应自动重新触发 `on_connected`
- 跨版本互通测试：与 Go 版 syncthing 至少 1 次自动化验证
- Linux 72h 长跑无内存泄漏、无死锁

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

**维护人**：juice094
**下次刷新**：H-1 完成时或 v0.2.6 发布后
