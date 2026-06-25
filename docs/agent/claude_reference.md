---
type: Reference
title: Claude Code Reference
description: Claude Code 使用 syncthing-rust 仓库时的详细参考：构建命令、feature flags、daemon 生命周期、CLI 命令、TUI 快捷键、REST API 与配置。
resource: ./claude_reference.md
tags: [agent, claude, reference, build, tui, api, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# Claude Code 参考

> 本文档承载 [`CLAUDE.md`](../../CLAUDE.md) 中的详细操作参考。通用 Agent 约束见 [constraints.md](constraints.md)，测试要求见 [testing.md](testing.md)。

---

## 1. 构建与测试

### 1.1 完整 CI 检查（每次提交前运行）

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo audit
```

### 1.2 Release 构建

```bash
# Windows 桌面版（含托盘）
cargo build --release --bin syncthing --features tray

# Headless / 服务器（无托盘、无 Windows UI 代码）
cargo build --release --bin syncthing --no-default-features

# 辅助二进制
cargo build --release --bin syncthing-tray       # 薄 wrapper（Windows only，向后兼容）
cargo build --release --bin syncthing-monitor    # 进程/资源监控
cargo build --release --bin stress_test          # 72h 双节点耐久测试
cargo build --release --bin gen_test_config      # 生成双节点测试配置
cargo build --release -p syncthing-cli           # generate-cert, show-id, metrics-flush
cargo build --release -p syncthing-bench         # Criterion 基准测试
cargo build --release -p syncthing-mcp-bridge    # MCP stdio ↔ REST API bridge
```

### 1.3 交叉编译 Linux (musl)

```bash
cargo build --release --bin syncthing --no-default-features --target x86_64-unknown-linux-musl
```

### 1.4 运行单个测试

```bash
cargo test -p syncthing-net --lib -- session::tests::test_session_ping_pong
```

---

## 2. Feature Flags

| Feature | Default | 效果 |
|:---|:---|:---|
| `tray` | ✅ | Windows 系统托盘集成（`#![windows_subsystem = "windows"]`，Win32 `Shell_NotifyIconW`） |
| `websocket` | ❌ | 在 `syncthing-net` 中启用 WebSocket transport |

Headless builds 排除所有 Windows UI 代码。`-p syncthing-cli` / `-p syncthing-bench` / `-p syncthing-mcp-bridge` 是独立 workspace members，不受 syncthing 的 feature 影响。

---

## 3. Crate DAG

```
cmd/syncthing              # CLI (clap) + TUI (ratatui) + daemon 生命周期 + tray + 辅助 bin
  ├─ syncthing-api         # REST + WebSocket (axum), EventBus, handlers
  ├─ syncthing-net         # ConnectionManager, ParallelDialer, discovery, relay (DERP)
  ├─ syncthing-sync        # Scanner, Puller, IndexHandler, folder_model, watcher
  │    ├─ syncthing-db     # 元数据 + 块缓存（通过 sync::database 消费）
  │    └─ syncthing-fs     # 文件系统抽象（通过 sync 内部消费）
  ├─ syncthing-core        # Types, traits — 无内部依赖
  ├─ syncthing-versioner   # Simple (keep=N) + Staggered (时间窗口)
  ├─ bep-protocol          # Wire protocol encode/decode (prost), handshake
  └─ syncthing-test-utils  # MemoryPipe, TestNode（仅测试/开发）

cmd/syncthing-tray         # 15 行薄 wrapper — 启动同目录 syncthing.exe
cmd/syncthing-cli          # generate-cert, show-id, metrics-flush
cmd/syncthing-bench        # Criterion 基准测试
cmd/syncthing-mcp-bridge   # MCP stdio ↔ REST API bridge
```

### 3.1 `cmd/syncthing/src/bin/` 内的二进制

- `stress_test.rs` — 72h 双节点耐久测试，含 SHA-256 内容哈希一致性检查
- `monitor.rs` — 跨平台进程监控（RSS、CPU、日志增长、静默检测）
- `gen_test_config.rs` — 生成双节点压力测试配置

### 3.2 耦合规则

- `syncthing-core` — 无内部 crate 依赖（纯 traits + types）
- `syncthing-api` — 仅依赖 `syncthing-core`；禁止持有具体 `ConnectionManagerHandle` 或 `LocalDatabase`
- `cmd/syncthing` — 粘合所有 crate；托盘代码位于 `#[cfg(all(windows, feature = "tray"))]` 后
- 单文件软限制：600 行

---

## 4. Daemon 生命周期

入口点：`cmd/syncthing/src/main.rs`

1. `tui::daemon_runner::start_daemon()` — TLS、ConnectionManager、SyncService、NAT、relay pool、discovery
2. `api_server::start_api_server()` — 绑定 REST API（默认从 `config.gui.address` 读取，即 `127.0.0.1:8385`）
3. `startup.future.await` — 主事件循环（session 清理、重连检查）
4. 关闭：`watch::Sender<bool>` 传播到所有子系统（Ctrl+C、ConsoleCtrlEvent、REST API `/rest/system/shutdown`）

**API 在 daemon 启动后 30–90s 可用** — relay pool TLS 健康检查（100 relays）在绑定前运行。开发/测试时可在配置中设 `relays_enabled: false` 跳过。

---

## 5. CLI 命令

```
syncthing                      # 无参数：daemon + tray（Windows）或仅 daemon（Linux）
syncthing run                  # 仅 daemon（前台，无托盘）
syncthing tui                  # TUI 客户端（连接现有 daemon）
syncthing init                 # 交互式配置向导
syncthing status [--json]      # 通过 REST API 查询 daemon 状态
syncthing devices list         # 列出已配置设备及其在线状态
syncthing folders list [--status]  # 列出文件夹及同步状态
syncthing logs --tail N        # 查看日志文件最后 N 行
syncthing install-autostart    # Windows：注册 HKCU Run 键
syncthing uninstall-autostart  # Windows：移除 HKCU Run 键
```

---

## 6. Windows 桌面模式

`syncthing.exe`（无参数）启动 daemon + 进程内托盘图标。

```
Main thread (tokio)
  ├── daemon_runner::start_daemon()  ← BEP sync engine
  ├── api_server::start_api_server() ← REST API on :8385
  └── tokio::spawn(tray_status_loop) ← 5s 轮询，图标/提示更新

Background thread
  └── Win32 message loop（hidden window + Shell_NotifyIconW + context menu）
```

`syncthing-tray.exe` 是薄 wrapper，在同目录查找 `syncthing.exe` 并启动它。仅用于兼容现有快捷方式和自动启动注册表项。

托盘源码位于 `cmd/syncthing/src/`：
- `tray.rs` — Win32 `Shell_NotifyIconW`、隐藏窗口、右键菜单、图标/提示/通知
- `tray_api.rs` — 状态轮询用的 `DaemonClient` REST 客户端
- `build.rs` — 在 `OUT_DIR` 生成 32×32 硬盘 ICO

从托盘启动 TUI 使用多终端回退：Windows Terminal (`wt.exe`) → PowerShell → CMD，并用 `AllocConsole()` + `SetConsoleScreenBufferSize` (120×40) 调整控制台窗口。

---

## 7. TUI 快捷键

| 键 | 上下文 | 动作 |
|:---|:---|:---|
| `F5` | 全局 | 启动 / 停止 daemon |
| `Tab` / `←→` | 全局 | 切换标签页（Overview / Devices / Folders / Logs） |
| `l` | 全局 | 循环日志过滤级别（Error→Warn→Info→Debug→Trace） |
| `q` | 无弹窗 | 退出 TUI |
| `?` | 无弹窗 | 帮助覆盖层 |
| `Insert` / `a` | Devices / Folders 标签 | 新增项目 |
| `Enter` / `e` | Devices / Folders 标签 | 编辑选中项目 |
| `Delete` / `d` | Devices / Folders 标签 | 删除选中项目 |
| `i` | Folders 标签 | 用系统编辑器打开 `.stignore` |
| `↑↓` | 列表视图 | 导航项目 |

弹窗（Add/Edit 表单）：`Tab`/`↑↓` 导航字段，`Space` 切换设备复选框，`Ctrl+V` 粘贴，`Enter` 保存，`Esc` 取消。

---

## 8. 配置

默认：BEP `0.0.0.0:22001`，REST API `127.0.0.1:8385`。

配置路径：
- Windows：`%LOCALAPPDATA%/syncthing-rust/config.json`
- Linux：`~/.config/syncthing-rust/config.json`

关键选项：`global_announce_enabled`、`relays_enabled`、`transports: ["tcp"]`。

`GET /rest/health` 是 ping 端点 — `/rest/system/ping` 不存在。

---

## 9. 关键 REST API

Handler 模式：`Result<Json<T>, (StatusCode, String)>`。所有 handler 接收 `State<ApiState>`。

| 端点 | 说明 |
|:---|:---|
| `GET /rest/health` | daemon liveness check |
| `GET /rest/system/status` | my_id, uptime, folder_count |
| `GET /rest/system/connections` | `{device_id: {connected, address, ...}}` |
| `GET /rest/db/browse?folder=X&prefix=Y&levels=N` | tree browse |
| `GET /rest/db/file?folder=X&file=Y` | 单文件元数据 |
| `GET /rest/events/poll?since=N&limit=M` | REST long-poll（60s 超时） |
| `GET /rest/events` | WebSocket upgrade |
| `POST /rest/system/shutdown` | 优雅关闭 |

---

## 10. 测试状态

392 passed / 6 ignored / 0 failed（workspace total，含 unit + integration + doc-tests）。

Ignored：
- `test_two_node_single_file_sync`（并行测试负载下 ClusterConfig race 偶发超时）
- `test_udp_broadcast_roundtrip`（CI runner 上 UDP loopback broadcast 不稳定）
- `test_public_relay_pool_tls_health_check` / `test_query_public_stun_server`（需要外网）
- 两个需要外网的 STUN doc-tests

---

## 11. 事实登记

`docs/KNOWN_ISSUES.md` 是项目级权威缺陷跟踪器。验证声明时，它优先于 handoffs 和 NEXT_STEPS 文档。新缺陷必须登记在那里。
