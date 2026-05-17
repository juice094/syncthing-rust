# 格雷 72h 持久化测试指南

> 对应版本: [v0.1.0](https://github.com/juice094/syncthing-rust/releases/tag/v0.1.0)

---

## 一、测试环境准备

### 1.1 获取二进制

```bash
# 方式一：直接下载 Release 中的 syncthing.exe
# 方式二：从源码编译
git clone https://github.com/juice094/syncthing-rust.git
cd syncthing-rust
git checkout v0.1.0
cargo build --release -p syncthing
```

### 1.2 启动命令

```bash
# Windows (格雷当前环境)
.\syncthing.exe run --listen 0.0.0.0:22000 --device-name gray-node

# 或者使用 TUI 模式（便于观察）
.\syncthing.exe tui
```

### 1.3 配置文件位置

Windows: `%LOCALAPPDATA%\syncthing-rust\config.json`

---

## 二、测试项目

### 测试 1：长时间连接稳定性（核心）

**目标**: 验证 Rust 节点与 Go 节点在 72 小时内保持连接，不出现无故断开。

**步骤**:
1. 启动 Rust 节点，配置 Go 节点为远程设备
2. 启动 Go Syncthing（格雷远程）
3. 保持两者运行 72 小时，期间**不人工重启**
4. 每小时记录一次连接状态

**通过标准**:
- 主动断开（如网络故障、重启）之外，不应出现 `Session ended` 日志
- 若出现断开，应在 5 分钟内自动重连成功

---

### 测试 2：文件同步完整性

**目标**: 双向文件同步 100% 正确。

**步骤**:
1. 在 Rust 侧文件夹创建 `test_push.txt`，写入随机内容
2. 等待 Go 侧接收，验证 MD5 一致
3. 在 Go 侧修改 `test_pull.txt`，等待 Rust 侧接收
4. 验证两者文件内容、时间戳、权限一致
5. **压力测试**: 连续创建/修改/删除 100 个小文件（1KB-1MB），验证最终状态一致

**通过标准**:
- 零文件丢失
- 零文件内容不一致
- 删除操作正确传播

---

### 测试 3：连接断开后的自动恢复

**目标**: 验证断线后自动重连，且退避时间合理。

**步骤**:
1. 正常建立连接
2. **模拟断网**: 断开 Rust 节点网络 30 秒，恢复后观察重连
3. **模拟 Go 节点重启**: 重启 Go Syncthing，观察 Rust 侧重连
4. 记录每次重连的延迟时间

**通过标准**:
- 网络恢复后 5 秒内开始重连尝试
- 重连退避时间应稳定：1s -> 2s -> 4s -> 8s ... 上限 5 分钟
- **不应出现**退避时间无限增长（如 5min -> 10min -> 20min...）

---

### 测试 4：配置持久化

**目标**: 配置在重启后完整保留。

**步骤**:
1. 通过 TUI 或 REST API 添加一个设备和一个文件夹
2. 记录设备 ID 和文件夹 ID
3. 完全关闭 Rust 节点
4. 重新启动
5. 验证设备和文件夹仍然存在

**通过标准**:
- `config.json` 中保留新增的设备/文件夹
- 重启后自动尝试连接已配置的设备

---

### 测试 5：内存与资源占用

**目标**: 72h 后内存占用稳定，无泄漏。

**步骤**:
1. 启动时记录内存基线（Windows Task Manager）
2. 每 12 小时记录一次内存占用
3. 72h 后记录最终内存

**通过标准**:
- 内存增长 < 50%（如基线 50MB，72h 后应 < 75MB）
- 无持续增长趋势（每次记录值应趋于平稳）

---

### 测试 6：REST API 可用性

**目标**: REST API 在长时间运行后仍然响应正常。

**步骤**:
1. 每小时调用以下端点：
   ```bash
   curl http://127.0.0.1:8385/rest/system/status
   curl http://127.0.0.1:8385/rest/system/connections
   curl "http://127.0.0.1:8385/rest/db/status?folder=<your-folder>"
   ```
2. 验证返回 JSON 格式正确，无 500 错误

**通过标准**:
- 所有端点 100% 可用
- 响应时间 < 100ms

---

## 三、需要保留的日志

### 3.1 运行日志（最重要）

启动时**务必**重定向日志到文件：

```bash
# Windows PowerShell
.\syncthing.exe run --listen 0.0.0.0:22000 2>&1 | Tee-Object -FilePath "syncthing-log-$(Get-Date -Format 'yyyyMMdd-HHmmss').txt"

# 或者使用 tracing 环境变量设置日志级别
$env:RUST_LOG="info,syncthing_net=debug,syncthing_sync=debug"
.\syncthing.exe run --listen 0.0.0.0:22000 2>&1 | Tee-Object -FilePath "syncthing-debug.log"
```

### 3.2 日志中需要重点关注的行

请保留**完整日志文件**（不要过滤），但如果需要快速定位问题，搜索以下关键词：

| 关键词 | 含义 | 正常情况 |
|--------|------|---------|
| `Session ended` | BEP 会话结束 | 极少出现（仅网络故障或手动断开） |
| `Scheduling reconnect` | 计划重连 | 断开后应出现，退避时间合理 |
| `retry_count=` | 重试计数 | 连接成功后应重置为 0 |
| `via race resolution` | 连接竞争解决 | 偶发出现，属于正常行为 |
| `Heartbeat timeout` | 心跳超时 | **不应频繁出现**（> 1次/小时则异常） |
| `Failed to dial` | 拨号失败 | 偶发可接受，但不应持续失败 |
| `ERROR` / `error` | 错误 | **任何 ERROR 都需记录并报告** |
| `panic` | 崩溃 | **绝不应出现** |
| `Folder loops started` | 文件夹监控启动 | 启动时正常 |
| `Config saved` | 配置保存 | TUI 操作后正常 |

### 3.3 系统资源日志（可选但推荐）

每小时记录一次：

```powershell
# 保存到 resources.log
"$(Get-Date) - Memory: $((Get-Process syncthing).WorkingSet64 / 1MB) MB, Handles: $((Get-Process syncthing).Handles)" | Add-Content resources.log
```

### 3.4 配置文件快照

测试开始前和结束后，各保存一份 `config.json`：

```powershell
Copy-Item "$env:LOCALAPPDATA\syncthing-rust\config.json" "config-before.json"
# 72h 后
Copy-Item "$env:LOCALAPPDATA\syncthing-rust\config.json" "config-after.json"
```

---

## 四、问题报告模板

若发现异常，请按以下格式报告：

```
**现象**: （简要描述）
**时间**: （发生的具体时间）
**日志片段**: （相关日志的 10-20 行上下文）
**复现步骤**: （如何再次触发）
**预期行为**: （应该发生什么）
**实际行为**: （实际发生了什么）
```

---

## 五、快速检查清单

- [ ] 二进制已下载/编译（v0.1.0）
- [ ] 启动命令已准备好（含日志重定向）
- [ ] Go 节点已配置为远程设备
- [ ] 测试文件夹已配置并共享
- [ ] REST API 可访问（`curl http://127.0.0.1:8385/rest/system/status`）
- [ ] 初始内存基线已记录
- [ ] 定时记录任务已设置（每小时/每12小时）

---

**祝测试顺利！有问题随时反馈。**
