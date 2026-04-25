# TUI Design — 美学与工程迭代方案

> **基线**: `e6d59c9`，TUI 已有 4 Tab + 弹窗 + F5 daemon 控制 + 设备/文件夹增删。
> **目标**: 从"功能可用"推进到"视觉愉悦 + 架构可扩展"。

---

## 一、现状诊断

### 1.1 美学层面

| 问题 | 具体表现 | 影响 |
|---|---|---|
| **配色单一** | 5 种颜色（Cyan/Green/Red/Gray/Yellow）硬编码 | 视觉疲劳，无品牌感 |
| **无动态反馈** | 同步中无 spinner，进度无进度条 | 用户不知道系统在做什么 |
| **信息层级混乱** | 设备列表中 ID/地址/状态等权重相同 | 核心信息（状态）不突出 |
| **日志无着色** | 所有日志纯白 | WARN/ERROR 无法一眼识别 |
| **弹窗简陋** | 单线边框，无阴影/高亮 | 缺乏"模态"感知 |
| **Overview 空洞** | 6 行静态文本 + 10 条日志预览 | 空间利用率低，无实时数据 |

### 1.2 工程层面

| 问题 | 具体表现 | 风险 |
|---|---|---|
| **ui.rs 单文件膨胀** | 346 行绘制逻辑，无组件拆分 | 新增 Tab = 持续膨胀，不可维护 |
| **颜色硬编码** | `Color::Cyan` 散落在 20+ 处 | 改主题 = 全局搜索替换 |
| **App 状态混合** | Config + UiState + RuntimeState 全在 `App` | 难以单元测试，状态变更难以追踪 |
| **事件耦合** | `events.rs` 直接操作 `app.config` | 业务逻辑泄漏到 UI 层 |
| **无动画框架** | 所有 widget 都是静态渲染 | 无法做平滑的加载/过渡效果 |

---

## 二、工程架构重构

### 2.1 状态分层（State Separation）

```rust
// app.rs
pub struct App {
    // === 持久化配置（用户编辑后需 save_config）===
    pub config: Config,
    
    // === 运行时状态（daemon 产生，只读给 UI）===
    pub runtime: RuntimeState,
    
    // === 纯 UI 状态（ ephemeral，重启后丢失）===
    pub ui: UiState,
}

pub struct RuntimeState {
    pub daemon_running: bool,
    pub connected_devices: Vec<DeviceId>,
    pub folder_statuses: HashMap<String, FolderRuntimeStatus>,
    pub device_statuses: HashMap<DeviceId, DeviceRuntimeStatus>,
    pub recent_events: VecDeque<BepSessionEvent>,
}

pub struct FolderRuntimeStatus {
    pub state: FolderStatus,      // Scanning / Syncing / Idle / Error
    pub local_files: usize,
    pub global_files: usize,
    pub local_bytes: u64,
    pub global_bytes: u64,
    pub completion: f64,          // 0.0 ~ 100.0
    pub need_bytes: u64,
    pub in_sync_devices: Vec<DeviceId>,
    pub last_scan: Option<DateTime<Utc>>,
    pub last_pull: Option<DateTime<Utc>>,
}

pub struct DeviceRuntimeStatus {
    pub connected: bool,
    pub address: Option<String>,
    pub in_bytes: u64,
    pub out_bytes: u64,
    pub shared_folders: Vec<String>,
    pub last_seen: Option<DateTime<Utc>>,
    pub completion_by_folder: HashMap<String, f64>,
}

pub struct UiState {
    pub tab: Tab,
    pub popup: Popup,
    pub device_selected: usize,
    pub folder_selected: usize,
    pub log_scroll: usize,
    pub theme: Theme,             // 可切换主题
    pub show_help: bool,
}
```

**收益**:
- `RuntimeState` 可由 daemon 通过 channel 异步更新，无需锁
- `UiState` 可完整序列化/恢复（窗口位置、主题偏好）
- 单元测试可独立测试 `RuntimeState` 计算逻辑

### 2.2 组件化目录结构

```
cmd/syncthing/src/tui/
├── mod.rs              # 入口、终端初始化、事件循环
├── app.rs              # App 结构 + 状态分层
├── state.rs            # RuntimeState / UiState 定义 + 更新逻辑
├── theme.rs            # Theme 定义 + 默认暗色主题
├── events.rs           # 键盘/鼠标事件路由（只转发，不处理业务）
├── actions.rs          # 业务动作（add_device、delete_folder 等）
├── widgets/            # 可复用组件
│   ├── mod.rs
│   ├── header.rs       # 顶部 Tab + 标题
│   ├── status_bar.rs   # 底部状态栏
│   ├── spinner.rs      # 动画加载指示器
│   ├── progress.rs     # 进度条
│   ├── log_line.rs     # 着色日志行
│   ├── device_row.rs   # 设备列表行
│   └── folder_row.rs   # 文件夹列表行
├── views/              # 页面级视图（每个 Tab 一个）
│   ├── mod.rs
│   ├── overview.rs     # Overview Tab
│   ├── devices.rs      # Devices Tab
│   ├── folders.rs      # Folders Tab
│   └── logs.rs         # Logs Tab
├── popups/             # 弹窗组件
│   ├── mod.rs
│   ├── add_device.rs
│   ├── add_folder.rs
│   ├── device_detail.rs    # 新增：设备详情
│   ├── folder_detail.rs    # 新增：文件夹详情
│   └── error.rs
└── daemon_runner.rs    # 保持不变
```

**收益**:
- 每个视图 < 150 行，可独立开发/测试
- 新增 Tab = 新增一个 `views/xxx.rs`，不碰现有代码
- 主题变更只需改 `theme.rs`

### 2.3 主题系统（Theme）

```rust
// theme.rs
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    
    // 语义颜色（不直接映射到 terminal color，而是业务含义）
    pub primary: Color,           // 品牌色（Syncthing 蓝）
    pub secondary: Color,         // 辅助色
    pub success: Color,           // 成功/在线
    pub warning: Color,           // 警告
    pub error: Color,             // 错误/离线
    pub info: Color,              // 信息提示
    pub muted: Color,             // 次要文本
    pub background: Color,        // 背景（暗色主题下不用，但为未来亮色预留）
    pub surface: Color,           // 卡片/面板背景
    pub border: Color,            // 边框
    pub border_focused: Color,    // 聚焦边框
    pub text_primary: Color,      // 主要文本
    pub text_secondary: Color,    // 次要文本
    
    // 样式快捷方式
    pub style_online: Style,
    pub style_offline: Style,
    pub style_syncing: Style,
    pub style_scanning: Style,
    pub style_idle: Style,
    pub style_error: Style,
    pub style_header: Style,
    pub style_popup_border: Style,
    pub style_log_trace: Style,
    pub style_log_debug: Style,
    pub style_log_info: Style,
    pub style_log_warn: Style,
    pub style_log_error: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self::syncthing_dark()
    }
}

impl Theme {
    pub fn syncthing_dark() -> Self {
        Self {
            name: "Syncthing Dark",
            primary: Color::Rgb(40, 100, 200),       // #2864C8 品牌蓝
            secondary: Color::Rgb(80, 160, 220),     // 辅助亮蓝
            success: Color::Rgb(100, 200, 100),      // 柔和绿
            warning: Color::Rgb(240, 180, 60),       // 琥珀黄
            error: Color::Rgb(220, 80, 80),          // 柔和红
            info: Color::Rgb(120, 180, 240),         // 信息蓝
            muted: Color::Rgb(120, 120, 120),        // 灰
            background: Color::Black,
            surface: Color::Rgb(30, 30, 35),         // 深蓝灰面板
            border: Color::Rgb(60, 60, 70),          // 暗边框
            border_focused: Color::Rgb(40, 100, 200), // 品牌蓝边框
            text_primary: Color::Rgb(230, 230, 230),  // 主文本
            text_secondary: Color::Rgb(160, 160, 160), // 次文本
            // ... 预计算样式
        }
    }
}
```

**使用方式**:
```rust
let theme = &app.ui.theme;
let status_style = if connected { theme.style_online } else { theme.style_offline };
```

---

## 三、美学设计规范

### 3.1 配色方案：Syncthing Dark

参考官方 Syncthing WebUI 的暗色主题，但针对终端 256 色优化：

```
Primary:      #2864C8  (品牌蓝)      → 用于标题、聚焦边框、进度条
Secondary:    #50A0DC  (亮蓝)       → 用于高亮、链接
Success:      #64C864  (柔和绿)     → 在线、同步完成
Warning:      #F0B43C  (琥珀黄)     → 警告、扫描中
Error:        #DC5050  (柔和红)     → 离线、错误
Info:         #78B4F0  (信息蓝)     → 提示、说明文字
Muted:        #787878  (灰)         → 次要信息、时间戳
Surface:      #1E1E23  (深蓝灰)     → 面板背景
Border:       #3C3C46  (暗边框)     → 普通边框
Text Primary: #E6E6E6  (近白)       → 主要文本
```

### 3.2 布局网格（响应式）

终端宽度分为 12 列，各 Tab 按需分配：

```
Overview (>= 100 cols):
┌──────────────────────────────────────────────────────────────┐
│ [Overview] [Devices] [Folders] [Logs]          syncthing-rust │  ← 3 height
├──────────────────┬──────────────────┬────────────────────────┤
│                  │                  │                        │
│   Device Info    │  Folder Summary  │    Global Stats        │  ← 6 height
│   (3 cols)       │  (5 cols)        │    (4 cols)            │
│                  │                  │                        │
├──────────────────┴──────────────────┴────────────────────────┤
│                                                              │
│                     Live Activity Timeline                    │  ← min 0
│                     (最近事件 + 带宽图表)                      │
│                                                              │
└──────────────────────────────────────────────────────────────┘
│ F5: Run/Stop │ Tab: Switch │ ↑↓: Navigate │ Enter: Detail │ q: Quit │  ← 1 height
```

### 3.3 信息层级（Typography）

```rust
// 通过 Style 修饰符表达层级，而非仅颜色
enum TextLevel {
    H1,        // 标题: Bold + 品牌色
    H2,        // 子标题: Bold
    Body,      // 正文: 正常
    Caption,   // 说明: Dim + 灰色
    Mono,      // ID/路径: 等宽字体色
}
```

设备列表示例：
```
┌─ Devices ─────────────────────────────────────────────────────┐
│  ▸ 格雷 (IKOL33P...)  100.99.240.98:22000   ● 在线  [100%]   │  ← H2 + Body + 状态色
│    共享: test-folder, rust-sync-test   已同步                │  ← Caption
│                                                              │
│    ASITKFU...        127.0.0.1:22001        ● 离线          │
│    共享: test-folder   从未连接                              │
└───────────────────────────────────────────────────────────────┘
```

### 3.4 动态视觉元素

#### Spinner（同步中指示器）
```rust
pub struct Spinner {
    frames: Vec<&'static str>,  // ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
    interval_ms: u64,
}
```
用于：Scanning / Syncing / Connecting 状态。

#### 脉冲点（在线状态）
```
● 在线      ← 稳定绿色
◐ 同步中    ← 旋转的 half-circle（spinner 变体）
○ 离线      ← 灰色空心
```

#### 进度条（文件夹同步）
```
test-folder  [████████████████████░░░░░░░░░░]  67%  (134/200 files, 2.3 MB/s ↓)
```
使用 `Gauge` widget，已同步部分用品牌蓝，待同步部分用 surface 色。

#### 带宽 Sparkline
```
↓ In:  [▃▄▆▇█▇▆▄▃▂▁]  2.3 MB/s
↑ Out: [▁▂▃▄▆▇█▇▆▄▃▂]  512 KB/s
```
使用 ratatui 的 `Sparkline`，缓存最近 60 个数据点（1秒采样）。

### 3.5 日志着色

```rust
fn colored_log_line(line: &str, theme: &Theme) -> Line {
    let level = detect_log_level(line);  // 从内容解析 TRACE/DEBUG/INFO/WARN/ERROR
    let style = match level {
        LogLevel::Trace => theme.style_log_trace,
        LogLevel::Debug => theme.style_log_debug,
        LogLevel::Info  => theme.style_log_info,
        LogLevel::Warn  => theme.style_log_warn,
        LogLevel::Error => theme.style_log_error,
    };
    Line::styled(line, style)
}
```

同时解析 tracing 的 span 字段，给 `folder_id` / `device` 等字段加次级高亮。

### 3.6 弹窗设计

```
┌────────────────────────────────────────┐
│  Add Device                    ╭───╮   │  ← 双线边框 + 右上角装饰
│ ═════════════════════════════════════  │
│                                        │
│  Device ID:  [____________________]    │  ← 聚焦字段：品牌蓝边框 + 光标
│  Name:       [____________________]    │
│  Address:    [127.0.0.1:22001____]    │
│                                        │
│         [ Save ]  [ Cancel ]           │  ← 按钮区域
│                                        │
└────────────────────────────────────────┘
```

弹窗背景使用半透明遮罩（通过 `Clear` widget + 底层内容 dim 实现）。

---

## 四、迭代优先级

### Wave 1: 基础重构（工程优先）

| 任务 | 文件 | 工作量 | 收益 |
|---|---|---|---|
| 状态分层 (`RuntimeState` / `UiState`) | `app.rs` + `state.rs` | 半天 | 为所有后续功能打地基 |
| 主题系统 | `theme.rs` | 2 小时 | 统一配色，一键换肤 |
| 目录重组（widgets/views/popups） | 目录移动 | 2 小时 | 可维护性 |
| 日志着色 | `widgets/log_line.rs` | 1 小时 | 立竿见影的视觉改善 |

### Wave 2: 视觉升级（美学优先）

| 任务 | 文件 | 工作量 | 收益 |
|---|---|---|---|
| Overview 重新设计（Global Stats + Activity Timeline） | `views/overview.rs` | 1 天 | 从"空洞"到"信息 dashboard" |
| 设备/文件夹列表行组件 | `widgets/device_row.rs` + `folder_row.rs` | 半天 | 信息层级清晰 |
| Spinner + 进度条 | `widgets/spinner.rs` + `widgets/progress.rs` | 半天 | 动态反馈 |
| 弹窗视觉升级（双线边框 + 按钮） | `popups/*.rs` | 半天 | 模态感知 |

### Wave 3: 功能补齐

| 任务 | 文件 | 工作量 | 收益 |
|---|---|---|---|
| 设备详情弹窗（连接状态、共享文件夹、Completion） | `popups/device_detail.rs` | 半天 | 深度信息不挤占列表 |
| 文件夹详情弹窗（文件列表、同步进度、设备状态） | `popups/folder_detail.rs` | 半天 | 同上 |
| 带宽 Sparkline | `widgets/sparkline.rs` | 半天 | 实时带宽可视化 |
| 主题切换（暗色/高对比/自定义） | `theme.rs` + 快捷键 | 2 小时 | 无障碍支持 |

### Wave 4:  polish

| 任务 | 工作量 | 收益 |
|---|---|---|
| 帮助页（`?` 键呼出快捷键清单） | 2 小时 | 降低学习成本 |
| 窗口大小变化平滑重绘 | 1 小时 | 响应式体验 |
| 配置导入/导出向导 | 半天 | 用户体验 |

---

## 五、关键技术决策

### Q: 是否引入 `tui-rs` 的第三方 widget crate？

**A: 不引入**。ratatui 0.28+ 的自带 widget（Gauge, Sparkline, Chart, Table, List）已足够。引入第三方 crate 会增加编译时间和依赖风险。

### Q: 动画如何在不阻塞事件循环的情况下运行？

**A**: 利用已有的 250ms tick 节奏：
```rust
// mod.rs 的 run_app 循环中
if last_tick.elapsed() >= tick_rate {
    app.ui.frame_counter += 1;  // 每 250ms 递增
    // spinner 每 4 帧（1秒）循环一次
    last_tick = tokio::time::Instant::now();
}
```
所有动画基于 `frame_counter` 计算当前帧，无需额外定时器。

### Q: 运行时状态如何同步？

**A**: daemon → TUI 通过 `tokio::sync::mpsc` channel：
```rust
// daemon_runner.rs 启动时创建 channel
tokio::spawn(async move {
    while let Some(event) = event_rx.recv().await {
        let _ = runtime_tx.send(RuntimeUpdate::from(event));
    }
});

// mod.rs 的 tick 中批量消费
while let Ok(update) = runtime_rx.try_recv() {
    app.runtime.apply(update);
}
```

---

## 六、验收标准

- [ ] TUI 启动后，不看代码也能判断各设备/文件夹的同步状态
- [ ] WARN/ERROR 日志在屏内一眼可辨
- [ ] 新增一个 Tab 无需修改 `ui.rs`，只需新增 `views/xxx.rs`
- [ ] 改配色方案只需修改 `theme.rs` 一处
- [ ] 设备/文件夹详情弹窗展示完整运行时状态（连接、completion、带宽）
