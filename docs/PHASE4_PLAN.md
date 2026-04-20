# Phase 4 Plan — TUI Hardening & Production Readiness

> **基线**: Phase 3 已完成（`e8882ca`），Push/Pull E2E 双向验证通过。
> **约束**: 本机（Windows）夜间断电，72h 压测移交格雷远程执行。
> **前端选型**: **TUI**（ratatui）作为唯一前端，不投入 GUI/WebUI。

---

## 为什么选 TUI，而不是 GUI/WebUI

| 维度 | TUI (ratatui) | GUI (egui/iced/tauri) | WebUI (React/Vue) |
|---|---|---|---|
| **现有基础** | ✅ 已有 4 个 Tab、弹窗、日志缓冲区 | ❌ 从零开始 | ❌ 从零开始 |
| **远程服务器友好** | ✅ SSH 直接运行 | ❌ 需要图形环境 | ⚠️ 需要端口转发 |
| **跨平台** | ✅ 纯终端，零依赖 | ⚠️ 打包复杂 | ⚠️ 浏览器兼容性 |
| **开发效率** | ✅ 1-2 天可出完整管理界面 | ❌ 1-2 周 | ❌ 1-2 周 |
| **资源占用** | ✅ 极小 | ❌ 大 | ❌ 中等 |
| **与 daemon 集成** | ✅ 同进程，直接调用 | ❌ IPC/HTTP 通信 | ❌ REST API 通信 |

**结论**: TUI 覆盖 90% 核心操作（设备配对、文件夹管理、状态查看），开发成本最低，且完美适配格雷的远程 Linux 服务器场景。

> 若未来确有 WebUI 需求，可基于现有 REST API 独立开发，作为可选组件。但不在 Phase 4 范围内。

---

## 4.1 TUI 功能补齐

当前 TUI 已有：
- 4 个 Tab（Overview / Devices / Folders / Logs）
- F5 启动/停止 daemon
- 弹窗系统（AddDevice / AddFolder / Error）
- 日志实时滚动
- 设备/文件夹列表展示

### 待实现

| 功能 | 说明 | 优先级 |
|---|---|---|
| **实时同步进度条** | 显示当前 Pull/Scan 进度 | P1 |
| **设备详情页** | 点击设备查看连接状态、共享文件夹、Completion | P1 |
| **文件夹详情页** | 点击文件夹查看文件列表、本地/全球状态 | P1 |
| **设备配对向导** | 输入 Device ID + 地址，自动保存配置 | P2 |
| **文件夹配置向导** | 选择本地路径、勾选共享设备 | P2 |
| **冲突文件标记** | 高亮显示 `.sync-conflict-*` 文件 | P3 |
| **实时带宽图表** | 基于 metrics 的收发速率（简单柱状图） | P3 |

### 验收标准
- [ ] TUI 可独立完成「添加设备 → 添加文件夹 → 启动同步 → 观察进度」全流程
- [ ] 无需手动编辑 `config.json`

---

## 4.2 协议兼容性收尾

| 问题 | 方案 | 优先级 |
|---|---|---|
| **连接循环**（双向拨号竞争） | 调研 Go Syncthing 的连接竞争解决逻辑，对齐 Rust 端行为 | P1 |
| **`.stignore`** | 集成 `syncthing-fs` crate 的 ignore 逻辑到 Scanner | P2 |
| **配置持久化** | `run` 命令将内存中的 Config 变化（新增设备/文件夹）写回 JSON | P2 |
| **Delta Index** | 验证 `IndexID` + `Sequence` 增量索引在长时间运行后的一致性 | P3 |

---

## 4.3 72h 压测（格雷远程执行）

详见 `PHASE3_PLAN.md` 3.4 节。

### 交付物
- Bash 压测脚本（`scripts/stress_test.sh`）
- 结果分析脚本（`scripts/analyze_stress.py`）

### 验收标准
- [ ] 72h 无不可恢复中断
- [ ] 所有文件变更 60 秒内传播
- [ ] 内存无泄漏（RSS 增长 < 10%）

---

## 4.4 生产打包

| 目标平台 | 方案 |
|---|---|
| **Linux (格雷)** | `cargo build --release` + systemd service 文件 |
| **Windows (本机)** | `cargo build --release` + 可选的 `cargo wix` MSI 安装包 |
| **配置文件路径** | Linux: `~/.config/syncthing-rust/` / Windows: `%LOCALAPPDATA%\syncthing-rust\` |

---

## 执行顺序

```
Week 1:
  4.1 TUI 功能补齐（设备/文件夹管理、同步进度）
  4.2 配置持久化 + .stignore

Week 2:
  4.2 连接循环根因修复
  4.3 压测脚本开发 + 提交格雷

Week 3:
  4.3 格雷执行 72h 压测
  4.4 生产打包（systemd service + MSI）
  Phase 4 报告
```

---

## 风险

| 风险 | 可能性 | 影响 | 缓解 |
|---|---|---|---|
| 连接循环修复复杂 | 中 | 高 | 参考 Go 源码 `lib/connections`，最小化改动 |
| `.stignore` 模式语法与 Go 不完全一致 | 中 | 中 | 仅支持核心语法（`*`, `**`, `!`），文档说明差异 |
| TUI 在 Windows 终端显示异常 | 低 | 低 | 已配置 `DisableMouseCapture`，ratatui 跨平台成熟 |
