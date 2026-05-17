# 工作交接：2026-05-16 Prometheus Metrics + v0.2.8 双节点测试

## ✅ 已完成

### 1. v0.2.8 双节点真实网络测试 — 通过
- **时间**：2026-05-16 22:39
- **环境**：Windows 11（校园网）↔ Ubuntu 22.04 VPS（Tailscale 100.127.13.26）
- **验证项**：
  - Tailscale 直连 ✅
  - TLS 1.3 握手 ✅
  - BEP Hello/ClusterConfig 交换 ✅
  - Windows → Linux 文件同步 ✅ `test-sync-v0.2.8.txt`
  - Linux → Windows 文件同步 ✅ `test-from-linux.txt`
  - Scanner 默认排除元数据 ✅
  - Watcher 实时检测 ✅

### 2. Prometheus `/metrics` 端点 — 代码已完成，部署验证阻塞
- **新增文件**：
  - `crates/syncthing-api/src/rest/metrics.rs` — Prometheus text format handler
- **修改文件**：
  - `crates/syncthing-api/src/rest/mod.rs` — 注册 `/metrics` 路由，白名单（无需 API key）
  - `docs/KNOWN_ISSUES.md` — 新增 §12 记录 v0.2.8 测试通过
- **暴露指标**（代码已实现）：
  | 指标名 | 类型 |
  |--------|------|
  | `syncthing_build_info` | gauge |
  | `syncthing_uptime_seconds` | gauge |
  | `syncthing_configured_devices` | gauge |
  | `syncthing_configured_folders` | gauge |
  | `syncthing_connected_devices` | gauge |
  | `syncthing_device_connected` | gauge |
  | `syncthing_total_bytes_sent` | counter |
  | `syncthing_total_bytes_received` | counter |
  | `syncthing_folder_files_total` | gauge |

---

## ⚠️ 当前阻塞点（P0）

### 阻塞点 1：API 服务器间歇性不启动
**症状**：
- 编译后的 binary 启动后，BEP 22001 正常监听，daemon 正常运行
- 但 REST API 端口 8385 **有时监听、有时不监听**
- 日志中 `syncthing::tui::daemon_runner: REST API enabled at 0.0.0.0:8385` 始终出现
- 但 `syncthing::api_server: REST API server listening on 0.0.0.0:8385` **间歇性出现**
- `api_server::start_api_server` 入口日志（`info!("start_api_server called...")`）也**间歇性出现**

**已验证**：
- `main.rs` 中 `Commands::Run` 分支确实包含 `api_server::start_api_server` 调用
- `config.json` 中 `gui.enabled = true`，`address = "0.0.0.0:8385"`
- cargo check / cargo build 均通过，无编译错误
- 同一 binary，同一启动命令，不同次启动行为不同

**根因推测**（待验证）：
- **A. Windows 单实例锁竞争** — `single_instance::acquire` 可能有残留锁文件，导致新进程行为异常
- **B. 端口抢占/TIME_WAIT** — 旧进程退出后端口处于 TIME_WAIT，新进程绑定失败但 fallback 逻辑未触发
- **C. `daemon_runner::start_daemon` 内部启动顺序** — `start_daemon` 可能在某些条件下提前返回或阻塞，导致 `main.rs` 后续代码未执行
- **D. Tokio 任务调度** — `startup.future` 和 `api_handle` 的竞争条件

**建议排查方向**（Claude 环境）：
1. 在 `main.rs` 的 `Ok(startup)` 分支第一行加 `info!` 日志，确认分支是否每次都被执行
2. 在 `api_server.rs` 的 `start_api_server` 入口和每个分支点加 `info!` 日志
3. 检查 `single_instance::acquire` 实现，确认是否有残留 pid 文件导致异常
4. 尝试将 API 启动逻辑从 `main.rs` 移到 `daemon_runner.rs` 内部，与 daemon 同生命周期启动
5. 或者：在 `daemon_runner.rs` 中直接启动 API 服务器，而不是在 `main.rs` 中分离启动

### 阻塞点 2：Windows 进程管理困难
**症状**：
- `pkill`、`taskkill`、`Stop-Process -Force` 均不能可靠终止 syncthing 进程
- 旧进程残留导致新 binary 部署后，实际运行的仍是旧 binary
- 多次出现 "curl 返回 404/000 但 binary 已更新" 的困惑

**已验证**：
- PID 32496 的旧进程（22:28 启动）在多次 kill 尝试后仍存活
- 最终通过 PowerShell `Stop-Process` 成功终止
- 但后续启动仍出现新旧进程混淆

**建议**：
- 在 `daemon_runner.rs` 或 `main.rs` 中添加 `--stop` 子命令，优雅终止同配置目录的进程
- 或者：启动前显式检查并终止同配置目录的旧进程

### 阻塞点 3：编译/部署迭代周期长
**症状**：
- release build：~2m 30s（UI 依赖 ratatui/image/arboard 编译慢）
- debug build：~30s
- 每次验证需要：编译 → 杀旧进程 → 复制 binary → 启动 → 等 API 就绪 → 测试
- 单次完整迭代 3-5 分钟

**建议**：
- 使用 `cargo check` 快速验证编译错误
- 使用 `cargo test -p syncthing-api` 在单元测试中验证 `/metrics` 路由
- 考虑为 API server 编写独立的 smoke test，无需启动完整 daemon

---

## 📋 待办事项（Claude 环境推进）

| 优先级 | 任务 | 说明 |
|--------|------|------|
| P0 | 解决 API 服务器间歇性不启动 | 定位根因并修复，确保 `/metrics` 稳定可用 |
| P0 | 部署 metrics 到格雷侧 | 格雷侧编译 Linux release binary，替换并验证 `/metrics` |
| P1 | 启动 72h 耐久测试 | 双节点保持运行，Prometheus 每 30s scrape，监控稳定性 |
| P1 | 版本号更新到 v0.2.9 | metrics 功能完成后打 tag，更新 CHANGELOG |
| P2 | 补充 metrics 指标 | bytes_transferred_rate、errors_total、reconnect_total 等 |

---

## 🔧 关键文件状态

```
crates/syncthing-api/src/rest/mod.rs        ← /metrics 路由 + middleware 白名单
crates/syncthing-api/src/rest/metrics.rs     ← Prometheus handler（已实现）
crates/syncthing-api/Cargo.toml              ← 无新增依赖（手动 text format）
cmd/syncthing/src/api_server.rs              ← 添加调试日志（可清理）
cmd/syncthing/src/main.rs                    ← 添加调试日志（可清理）
docs/KNOWN_ISSUES.md                         ← §12 已更新
docs/handoffs/HANDOFF_2026-05-16.md          ← 本文档
```

## 📝 格雷侧信息

- **Device ID**：`AKLPRHA-CRT6HA4-2RXCDP5-EUQ4VVR-QUQ2PWK-F5357SQ-NZCIUEC-NPLOSA6`
- **Tailscale IP**：`100.127.13.26`
- **同步文件夹**：`/home/hadoop/syncthing-rust/sync`
- **配置目录**：`/home/hadoop/syncthing-rust/sync-config`
- **API 端口**：`8385`
- **状态**：syncthing v0.2.8 运行中，等待宿侧部署 metrics 版本后同步更新

---

*交接时间：2026-05-16 23:30 UTC+8*
*交接人：Kimi Code CLI*
*接收环境：Claude*
