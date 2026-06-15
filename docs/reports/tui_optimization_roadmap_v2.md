# TUI 优化路线图 v2

> 状态：自由规划与探索阶段  
> 日期：2026-06-15  
> 基线：syncthing-rust v3.0.3，TUI 首帧底行问题已修复

---

## 1. 当前基线

### 1.1 已通过检查

- `cargo fmt --all`：通过
- `cargo clippy -p syncthing --all-targets -- -D warnings -W clippy::await_holding_lock`：0 warning
- `cargo test -p syncthing`：22 unit + 6 E2E passed，1 ignored

### 1.2 当前文件规模（按行数排序）

```text
  673  cmd/syncthing/src/tui/events.rs        ⚠️ 超过 600 行软限制
  586  cmd/syncthing/src/tui/daemon_runner.rs  ⚠️ 接近限制
  515  cmd/syncthing/src/tui/runner.rs         ⚠️ 偏大
  297  cmd/syncthing/src/tui/daemon_controller.rs
  290  cmd/syncthing/src/tui/app.rs
  256  cmd/syncthing/src/tui/forms/render.rs
  236  cmd/syncthing/src/tui/views/overview.rs
  218  cmd/syncthing/src/tui/widgets/log_line.rs
  201  cmd/syncthing/src/tui/discovery_tasks.rs
  182  cmd/syncthing/src/tui/forms/mod.rs
  178  cmd/syncthing/src/tui/relay_listener.rs
  140  cmd/syncthing/src/tui/ui.rs
  124  cmd/syncthing/src/tui/theme.rs
  121  cmd/syncthing/src/tui/log_loader.rs
  109  cmd/syncthing/src/tui/popups/help.rs
  104  cmd/syncthing/src/tui/views/folders.rs
   99  cmd/syncthing/src/tui/widgets/progress.rs
   99  cmd/syncthing/src/tui/nat_tasks.rs
   88  cmd/syncthing/src/tui/index_dispatcher.rs
   87  cmd/syncthing/src/tui/views/logs.rs
   87  cmd/syncthing/src/tui/session_logger.rs
   75  cmd/syncthing/src/tui/forms/handler.rs
   69  cmd/syncthing/src/tui/views/devices.rs
   55  cmd/syncthing/src/tui/widgets/status_bar.rs
   44  cmd/syncthing/src/tui/widgets/mod.rs
   43  cmd/syncthing/src/tui/popups/confirm.rs
   43  cmd/syncthing/src/tui/popups/error.rs
   43  cmd/syncthing/src/tui/mod.rs
   31  cmd/syncthing/src/tui/constants.rs
   29  cmd/syncthing/src/tui/widgets/header.rs
    4  cmd/syncthing/src/tui/popups/mod.rs
    3  cmd/syncthing/src/tui/views/mod.rs
```

### 1.3 已实现的优化（相比 v1 计划）

| 优化项 | 状态 | 位置 |
|--------|------|------|
| 拆分 `mod.rs` | ✅ 完成 | `runner.rs` / `daemon_controller.rs` / `log_loader.rs` |
| Overview 总览卡片 | ✅ 完成 | `views/overview.rs` |
| Logs 滚动 | ✅ 完成 | `events.rs` / `views/logs.rs` |
| 搜索 / 过滤弹窗 | ✅ 完成 | `Popup::Search` / `Popup::Filter` |
| 删除确认对话框 | ✅ 完成 | `Popup::Confirm` / `popups/confirm.rs` |
| 日志级别切换 | ✅ 完成 | `'l'` 快捷键 |
| status bar 分隔线 | ✅ 完成 | `ui.rs` / `widgets/status_bar.rs` |
| 首帧底行修复 | ✅ 完成 | `runner.rs` synchronized update + startup resize |

---

## 2. 代码结构问题（P0）

### 2.1 `events.rs` 673 行 —— 最优先拆分

当前职责：

- 全局按键分发（`handle_key`）
- Tab 模式快捷键（`handle_tab_key`）
- Logs 页快捷键（`handle_logs_key`）
- 弹窗按键：Search / Filter / Confirm / Form
- 表单提交：Device / Folder / .stignore 编辑器
- 配置持久化

**建议拆分：**

```text
cmd/syncthing/src/tui/
├── events/
│   ├── mod.rs          # handle_event 入口
│   ├── global.rs       # q / F5 / ? / l / Tab 切换
│   ├── logs.rs         # Logs tab 快捷键
│   ├── search.rs       # Search / Filter 弹窗
│   ├── confirm.rs      # Confirm 弹窗
│   ├── editor.rs       # 打开 notepad/nano 编辑 .stignore
│   └── actions.rs      # add/edit/delete/submit 业务动作
```

目标：`events/mod.rs` 200 行以内，子模块各 < 200 行。

### 2.2 `daemon_runner.rs` 586 行 —— 第二大文件

当前职责：

- 启动 daemon 主循环
- 连接管理器创建
- 传输层注册（TCP / WebSocket / DERP / Proxy）
- 发现任务启动
- REST API 服务器启动

**建议拆分：**

```text
cmd/syncthing/src/tui/
├── daemon_runner/
│   ├── mod.rs          # start_daemon 入口
│   ├── transports.rs   # TCP/WS/DERP/Proxy 注册
│   ├── discovery.rs    # LAN/Global/STUN/UPnP 启动
│   └── api_server.rs   # REST API 启动
```

### 2.3 `runner.rs` 515 行 —— 终端初始化与事件循环耦合

当前职责：

- `run_tui`：终端设置、配置加载、App 创建
- `run_app`：尺寸稳定、强制重绘、主事件循环、daemon 生命周期
- WinAPI helper 函数

**建议拆分：**

```text
cmd/syncthing/src/tui/
├── runner.rs           # run_tui 入口
├── app_loop.rs         # run_app 主循环
└── terminal_winapi.rs  # Windows 专用 helper（cfg windows）
```

---

## 3. 交互体验优化（P1）

### 3.1 Device ID 自动格式化

当前 `forms/mod.rs` 已支持 Device ID 输入，但无自动 `-` 分隔。

**建议：**

- 在 `FormField` 中增加 `auto_format: Option<fn(&str) -> String>`。
- Device ID 字段输入时，每 7 字符自动插入 `-`。
- 粘贴长字符串时自动格式化并截断到 56 字符。
- 提交前校验长度与字符集，错误显示在表单底部而非弹窗。

### 3.2 表单验证失败保留状态

当前验证失败弹出 `Popup::Error`，关闭后表单消失。

**建议：**

- `FormState` 增加 `error: Option<String>`。
- 在 `forms/render.rs` 底部绘制红色错误行。
- 按 `Esc` 关闭表单时才丢弃输入，`Enter` 提交失败时保留。

### 3.3 F5 状态提示更明确

当前外部 daemon 时显示 `External`，不够直观。

**建议：**

| 场景 | 状态栏 F5 提示 |
|------|----------------|
| 本 TUI 启动 daemon | `F5 Stop` |
| 外部 daemon + 托盘 IPC | `F5 trayctl` |
| 外部 daemon 无托盘 | `F5 disabled` |
| daemon 未运行 | `F5 Run` |

### 3.4 Logs 搜索高亮

当前 `Popup::Search` 仅记录 query，未高亮匹配行。

**建议：**

- 在 `views/logs.rs` 中根据 `app.log_search` 高亮匹配子串。
- `n/N` 跳转到下一个/上一个匹配。
- 无匹配时显示 `No matches`。

---

## 4. 性能与健壮性优化（P1/P2）

### 4.1 大日志文件反向 tail（P1 → P0）

当前 `log_loader::tail_lines` 使用 `read_to_string`，大文件会阻塞 TUI。

**建议：**

- 使用 `std::fs::File::seek(SeekFrom::End(-chunk_size))` 从末尾按块读取。
- 限制最大读取字节数（如 1 MiB），超出显示 `[...truncated]`。
- 避免读取整个文件，保持 O(需要行数 × 平均行长) 而非 O(文件大小)。

### 4.2 日志缓冲区空轮询

当前 `memory_buffer.take_lines_filtered` 每 tick 调用，即使无新日志。

**建议：**

- `MemoryBuffer` 增加 `has_new()` 原子标志或 `watch` 通知。
- TUI 只在有新日志时刷新日志视图，降低 CPU 占用。

### 4.3 托盘模式轮询抖动

托盘模式每 ~2s 轮询 REST API，瞬时不通即显示 Stopped。

**建议：**

- 连续 2 次失败才标记 Stopped。
- 或增加 "degraded" 中间状态。

---

## 5. 视觉与可访问性优化（P2/P3）

### 5.1 Header Tab 高亮增强

当前高亮仅加粗，宽终端下不明显。

**建议：**

- 当前 Tab 使用 `theme.primary` 背景 + 白色文字。
- 非当前 Tab 使用 `theme.border`。

### 5.2 状态符号颜色盲友好

| 状态 | 符号 |
|------|------|
| Online | `●` 或 `↑` |
| Offline | `○` 或 `↓` |
| Scanning | `⟳` |
| Error | `✗` |

### 5.3 日志着色精确匹配

当前 `log_line.rs` 用 `contains(level)` 会误匹配内容中的关键字。

**建议：**

- 严格匹配 tracing 前缀：`YYYY-MM-DDTHH:MM:SS.NNNNNNZ LEVEL target:`。
- 回退时才用 contains。

### 5.4 主题切换（P3）

- `t` 键切换暗色/亮色/高对比度。
- 持久化到 `config.json` 的 `gui.theme`。

---

## 6. 推荐实施路线图

### Phase 1：代码结构（1-2 天）

1. 拆分 `events.rs` → `events/` 子模块。
2. 拆分 `daemon_runner.rs` → `daemon_runner/` 子模块。
3. 拆分 `runner.rs` → `runner.rs` + `app_loop.rs` + `terminal_winapi.rs`。
4. 每次拆分后运行 `cargo fmt / clippy / test`。

### Phase 2：交互体验（2-3 天）

1. Device ID 自动格式化。
2. 表单验证失败保留状态。
3. F5 状态提示细化。
4. Logs 搜索高亮。

### Phase 3：性能健壮性（2-3 天）

1. 大日志反向 tail。
2. 日志缓冲区空轮询优化。
3. 托盘轮询抖动抑制。

### Phase 4：视觉 polish（可选）

1. Header Tab 高亮增强。
2. 状态符号。
3. 日志精确着色。
4. 主题切换。

---

## 7. 风险与约束

- **AGENTS.md 文件规模限制**：单文件 600 行软限制。拆分后必须满足。
- **crate 边界**：TUI 代码在 `cmd/syncthing` 内，不受 `syncthing-core` 红线约束，但仍需保持 `syncthing-api` 通过 trait 交互。
- **测试要求**：网络/发现相关改动需用 `TestNode` 双实例验证；UI 交互改动需配套单元测试或 E2E 测试。
- **Windows 终端兼容性**：任何 TUI 启动/尺寸/清屏改动都必须在 Windows Terminal 和 conhost 下验证。

---

## 8. 下一步建议

如果继续推进，建议按 Phase 1 开始，先拆分 `events.rs`。这是当前最迫切需要处理的文件，也是风险最低的结构优化——只是代码移动，不引入新行为。完成后立即验证 clippy/test 基线，再进入 Phase 2。
