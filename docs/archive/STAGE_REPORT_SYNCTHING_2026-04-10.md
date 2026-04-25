# syncthing-rust-rearch 阶段性报告

**日期**: 2026-04-10  
**提交人**: Kimi Code CLI

---

## 一、本次完成的核心功能

### 1. 同步基准测试（syncbench）
- `cmd/syncthing/src/syncbench.rs`: 新增 `syncbench` 子命令，支持 `Small`、`Medium`、`Large`、`Mixed` 四种场景。
- 自动生成测试数据到 `source_dir`，验证 `target_dir` 的同步结果，输出 JSON 报告。
- 已通过编译并集成到 CLI（`syncthing syncbench <scenario>`）。

### 2. 可观测性：MetricsCollector + CSV 导出
- `crates/syncthing-net/src/metrics.rs`: 新增全局 `MetricsCollector`，记录：
  - TLS 握手耗时
  - BEP 消息收发事件
  - 连接建立/重连事件
- `cmd/syncthing/src/main.rs`: 新增 `metrics-flush <path>` 子命令，可将指标导出为 CSV。

### 3. 配置持久化增强
- `syncthing_core::types::Config`: 扩展了 `listen_addr` 和 `device_name` 字段。
- `cmd/syncthing/src/main.rs`: 守护进程启动时优先从 `config_dir/config.json` 加载配置，CLI 参数可覆盖，保存时自动回写。
- 验证了 `save_config` / `load_config` 的 round-trip 正确性。

### 4. 一键验证脚本（今日新增）
- 新增 `syncthing_validation.ps1`（根目录），Git 提交 `d2c9132`。
- 脚本覆盖：编译检查 → 证书生成 → Go/Rust 双向互信配置 → daemon 启动 → BEP 握手验证 → 端到端文件同步检测 → metrics 导出 → 安全清理。

### 5. 编译与基础可用性验证
- `cargo build --release -p syncthing`: 通过（20 warnings，0 errors）
- `cargo test` (workspace): 全绿（28 passed）
- CLI 基本功能实测通过：
  - `generate-cert` ✅
  - `show-id` ✅
  - `run` (daemon 起停) ✅
  - `tui` (初始化与事件循环) ✅
  - `metrics-flush` ✅
  - `syncbench small` ⚠️ 预期行为（无 live peer 时 files_missing）

---

## 二、今日最终验证结果（预演）

通过 `syncthing_validation.ps1` 进行了完整预演，结论如下：

| 检查项 | 结果 | 说明 |
|--------|------|------|
| 编译 release | ✅ 通过 | 0 errors |
| 证书生成/显示 | ✅ 正常 | Device ID 可正确生成与读取 |
| Rust daemon 启动 | ✅ 正常 | 成功监听 `127.0.0.1:22000` |
| Go peer 启动 | ✅ 正常 | 成功监听 `127.0.0.1:22001`，REST API 可用 |
| **BEP 握手/连接建立** | ✅ **30 秒内成功** | Go REST API 确认 `connected: true` |
| **端到端文件同步** | ⚠️ **20 秒内未完成** | 文件写入 Rust shared-folder 后，未在 20s 内出现在 Go shared-folder |
| metrics-flush | ✅ 正常 | CSV 成功导出（当前无网络事件记录，因为连接时间过短） |

### 端到端同步未闭环分析
- **BEP 协议层握手已成功**：TLS 建立、ClusterConfig 交换、Index 发送均正常。
- **文件未同步的原因**：当前 `syncthing-rust` 的 `folder_model` pull loop 虽在运行，但实际的“接收对端 Index → 比对缺失 → 发起 Request → 写入本地文件”完整链路尚未完全闭环。这是**已知的开发阶段状态**，非脚本故障。
- **明日重点**：需要让 sync service 在收到对端文件索引后，正确触发 block request 并将数据写入目标目录。

---

## 三、测试状态

- 单元测试: **28 passed, 0 failed**
- 编译状态: **release 通过**
- 手动 CLI 验证: **全部可用**
- BEP 握手验证: **已通过预演**
- 端到端文件同步: **未闭环（待开发）**

---

## 四、待测试 / 待完成事项

### 明日优先（基于今日预演结论）
1. [ ] **文件夹同步闭环**：修复/补齐 `syncthing-rust` 在收到对端 `Index` 后的 block request 与本地文件写入逻辑
2. [ ] **端到端 syncbench 成功**：在 live peer 环境下运行 `syncbench small`，确认 `success: true`
3. [ ] **metrics 数据完整性**：延长连接时间或在 BEP session 中触发更多事件，确保 `metrics-flush` 的 CSV 包含有效记录
4. [ ] **TUI 配置管理器可用性**：在 TUI 中新增/删除 folder 和 device，验证配置回写

### 短期（本周内）
5. [ ] BEP 协议与 Go 原版的兼容性压测（长时间运行、大文件、断网重连）
6. [ ] `ConnectionManager` 在 Windows 上的 UPnP / NAT-PMP 发现成功率测试
7. [ ] 错误日志与 tracing 级别调优（减少 noise，保留关键网络事件）

### 中期（架构投资）
8. [ ] 评估 `iroh` 的 `Endpoint` + `ALPN Router` 作为未来 QUIC 传输层替代方案
9. [ ] 将 `MetricsCollector` 与 `tracing` / OpenTelemetry 生态打通
