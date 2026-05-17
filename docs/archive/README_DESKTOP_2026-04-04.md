# Syncthing Rust

Syncthing 的 Rust 重构实现 - 一个连续文件同步程序。

> ⚠️ **开发状态**: 本项目处于实验性开发阶段，**不建议用于生产环境**。

## 功能状态概览

| 模块 | 状态 | 测试 | 说明 |
|------|------|------|------|
| syncthing-core | ✅ 可用 | 32 通过 | 核心类型、版本向量、错误定义 |
| syncthing-fs | ✅ 可用 | 63 通过 | 文件系统、扫描器、监控、忽略模式 |
| syncthing-db | ✅ 可用 | 36 通过 | KV 存储、块缓存、元数据存储 |
| syncthing-api | ✅ 可用 | 24 通过 | REST API、事件系统、配置管理 |
| bep-protocol | ✅ 可用 | 29 通过 | BEP 消息编解码、连接管理 |
| syncthing-net | ✅ 可用 | 48+2 通过 | TCP/TLS/发现/连接管理/request-response |
| syncthing-sync | ✅ 可用 | 41+2 通过 | 同步引擎、索引、拉取/推送、冲突解决 |
| CLI | ✅ 可用 | - | init / run / scan / generate |

**总测试数**: 280+ 全部通过（含端到端集成测试）

## 已实现功能

### 1. 文件系统 (syncthing-fs)
- ✅ 异步文件读写操作
- ✅ SHA-256 块哈希计算
- ✅ 目录递归扫描
- ✅ 文件系统监控（创建/修改/删除事件）
- ✅ `.stignore` 模式匹配
- ✅ 跨平台路径处理

### 2. 数据存储 (syncthing-db)
- ✅ 分层存储（内存缓存 + 磁盘 sled）
- ✅ 内容寻址块存储
- ✅ 文件夹索引管理
- ✅ 元数据存储

### 3. REST API (syncthing-api)
- ✅ 文件夹管理（CRUD）
- ✅ 设备管理（CRUD）
- ✅ 系统状态查询
- ✅ 扫描触发
- ✅ 暂停/恢复控制
- ✅ WebSocket 事件流
- ✅ 健康检查端点 (`/rest/health`)

### 4. 网络层 (syncthing-net)
- ✅ TCP + TLS 1.3 + mTLS 双向认证
- ✅ 本地发现（UDP 广播/多播，端口 21027）
- ✅ 全局发现（Syncthing 官方服务器）
- ✅ 连接管理器（连接池、keepalive、超时清理）
- ✅ BEP Hello 握手（与 Go Syncthing v2.0.15 兼容）
- ✅ **Request/Response 块传输**（2026-04-11 修复完成）
- ⚠️ Iroh QUIC P2P 消息收发未完成（stub）

### 5. 同步引擎 (syncthing-sync)
- ✅ 索引管理（本地/远程）
- ✅ 拉取调度（按优先级并发下载块）
- ✅ 推送响应（处理远程块请求）
- ✅ 版本向量冲突检测
- ✅ 冲突副本创建（`.sync-conflict-...`）
- ✅ 文件夹扫描与同步状态机
- ✅ **端到端文件同步验证通过**（集成测试 `test_minimal_local_sync`）

### 6. BEP 协议 (bep-protocol)
- ✅ 消息编解码（protobuf / prost）
- ✅ Hello 握手流程
- ✅ 连接抽象
- ✅ Request/Response ID 关联

## 快速开始

### 构建

```bash
cd syncthing-rust-rearch

# 构建所有组件
cargo build --release

# 运行测试
cargo test --workspace
```

### CLI 使用

```bash
# 初始化配置（自动生成证书和设备ID）
./target/release/syncthing.exe init

# 生成设备ID
./target/release/syncthing.exe generate

# 扫描文件夹
./target/release/syncthing.exe scan

# 启动完整同步服务（API + 网络同步 + 文件扫描）
./target/release/syncthing.exe run

# 启动 TUI 交互界面（推荐）
./target/release/syncthing.exe run --tui

# 仅扫描模式
./target/release/syncthing.exe run --scan-only

# 无 API 模式
./target/release/syncthing.exe run --no-gui
```

### TUI 操作说明

启动 `run --tui` 后，你可以通过终端界面实时监控同步状态：

- **左面板**：文件夹列表（显示状态：Idle / Scanning / Syncing / Error）
- **右上面板**：已知设备列表（● 在线 / ○ 离线）
- **右下面板**：最近事件日志
- **按键**：
  - `q` / `Esc` — 退出
  - `r` — 触发所有文件夹重新扫描
  - `p` — 触发所有文件夹拉取
  - `Tab` — 切换左/右面板焦点
  - `↑↓` — 导航选择

### API 测试

```bash
# 健康检查
curl http://127.0.0.1:8384/rest/health

# 获取系统状态
curl http://127.0.0.1:8384/rest/status

# 列出文件夹
curl http://127.0.0.1:8384/rest/folders

# 列出设备
curl http://127.0.0.1:8384/rest/devices
```

## 架构

```
┌─────────────────────────────────────────────────────────────┐
│                        syncthing-api                        │
│                    (REST API / WebSocket)                   │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────┼─────────────────────────────┐
│                             │                             │
▼                             ▼                             ▼
┌──────────────┐     ┌─────────────────┐     ┌──────────────┐
│syncthing-sync│     │   syncthing-net │     │syncthing-core│
│  (同步引擎)   │◄───►│  (P2P 网络层)    │     │  (核心类型)  │
└──────┬───────┘     └────────┬────────┘     └──────┬───────┘
       │                      │                     │
       ▼                      ▼                     ▼
┌──────────────┐     ┌─────────────────┐     ┌──────────────┐
│syncthing-fs  │     │  bep-protocol   │     │syncthing-db  │
│ (文件系统)   │     │   (BEP 协议)    │     │  (数据存储)   │
└──────────────┘     └─────────────────┘     └──────────────┘
```

## 已知限制

1. **无 Web UI**: 目前只有 REST API，没有 Web 界面。
2. **与原版 Syncthing 不完全兼容**: BEP Hello 握手和块传输已与 Go 版本验证通过，但完整的 ClusterConfig、版本协商等尚未充分测试。
3. **Global Discovery 偶发故障**: Windows 下 reqwest TLS 创建偶发失败，可通过静态地址绕过。
4. **Iroh QUIC 路径**: Iroh P2P 消息收发仍为 stub，当前同步依赖 TCP/TLS 路径。
5. **生产使用**: **不推荐**用于生产环境或重要数据。

## 开发状态

### 已完成
- 核心类型和错误处理
- 文件系统抽象和监控
- 数据存储层
- REST API 服务器
- BEP 协议编解码
- TCP/TLS 网络传输、设备发现、连接管理
- BEP Hello 握手与 Go Syncthing 兼容验证
- **端到端文件同步**（2026-04-11 修复完成，集成测试通过）

### 待完成
- Web UI 实现
- 与原版 Syncthing 的完整兼容测试
- 压力测试和性能优化
- NAT 穿透（UPnP/STUN）
- 中继服务器支持
- Iroh QUIC 路径收尾

## 已归档组件

以下组件已移至 `archive/2026-04-11-cleanup/`：

- ~~`syncthing-sync`~~ — **已恢复并修复**，重新加入工作区
- ~~`acceptance-tests`~~ — **已恢复**，placeholder 测试已替换为真实端到端测试

## 后续测试建议

### 1. 单元测试与集成测试
```bash
cargo test --workspace        # 280+ 测试（含端到端同步测试）
cargo test -p acceptance-tests # 仅运行端到端集成测试
```

### 2. 本地双设备同步测试（手动）
```bash
# 终端 A：启动设备 1
mkdir -p /tmp/syncthing-a
cargo run --release --bin syncthing -- init --config-dir /tmp/syncthing-a
# 编辑 config.toml 添加文件夹和共享设备，然后：
cargo run --release --bin syncthing -- run --tui --config-dir /tmp/syncthing-a

# 终端 B：启动设备 2
mkdir -p /tmp/syncthing-b
cargo run --release --bin syncthing -- init --config-dir /tmp/syncthing-b
# 编辑 config.toml 添加相同文件夹 ID 和对方设备 ID/地址，然后：
cargo run --release --bin syncthing -- run --tui --config-dir /tmp/syncthing-b
```
观察 TUI 中的文件夹状态变化和文件是否成功同步。

### 3. 与 Go 版 Syncthing 互通测试
- 启动 Go 版 Syncthing（`C:\Users\22414\dev\third_party\syncthing`）
- 在 Rust 版本的 `config.toml` 中添加 Go 版设备的 ID 和地址
- 运行 `syncthing.exe run --tui`，观察设备是否显示为在线
- 在共享文件夹中增删文件，验证双向同步

### 4. TUI 测试
- 启动 `run --tui`，按 `r` 观察扫描状态变为 Scanning 后恢复 Idle
- 按 `p` 观察有差异时状态变为 Syncing
- 按 `q` 正常退出，确认服务优雅停止

## 贡献

本项目采用 Agent Cluster 协作模式开发。各 crate 可能包含 `UNVERIFIED` 标记的代码，表示由子 Agent 生成，需要主控验证。

## 许可证

MIT License

## 相关文档

- [AGENT_CLUSTER_SPEC.md](./AGENT_CLUSTER_SPEC.md) - Agent 集群协作规范
- [PROJECT_STATUS.md](./PROJECT_STATUS.md) - 项目当前状态
- [DEVLOG.md](./DEVLOG.md) - 开发日志
