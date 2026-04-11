# 今日工作完成报告

**日期**: 2026-04-09  
**工作主题**: MVP 修复计划 Phase 1 & Phase 2 完成，首次通过真实 Go Syncthing 互操作测试  
**涉及范围**: `syncthing-rust-rearch` 全工作区

---

## 一、今日目标回顾

昨日（及之前）完成了 Wave 1~3 的大量组件开发，但代码审计暴露出项目存在**4 个致命断裂点**，导致无法与真实 Syncthing 节点互通。今日的核心目标是启动 **MVP 修复计划**，逐步打通这些断裂点，使项目从一个"库集合"向"可运行的 daemon"演进，并最终**与真实 Go Syncthing 完成握手验证**。

---

## 二、今日完成的具体工作

### 1. 代码审计与规划 ✅
- **上午**: 进行了只读代码审计，客观评估了各 crate 的真实完成度。
- **产出**: 写入 `MVP_RECOVERY_PLAN.md`，将修复工作拆分为 4 个任务：
  - NET-TCP-FIX
  - CMD-DAEMON
  - BEP-MESSAGES
  - SYNC-PULLER-NET

### 2. NET-TCP-FIX — TCP 传输路径全面修复 ✅

**问题**: TCP 路径存在三个致命 bug：没有 TLS、发送 JSON 而非 protobuf Hello、BEP 帧解析破坏流。

**修复内容**:
- `crates/syncthing-net/src/manager.rs`: `ConnectionManager::new` 现在接受外部 `Arc<SyncthingTlsConfig>`，不再私自重新生成证书
- `crates/syncthing-net/src/tcp_transport.rs`: 在 `connect_bep` 和 `listen` 中补上了完整的 `tokio_rustls` 客户端/服务端 TLS 握手
- `crates/syncthing-net/src/protocol.rs`: `HelloMessage` 从 JSON 编码彻底改为 `bep_protocol::handshake::Hello` 的 protobuf 编码（magic + 2-byte length prefix）
- `crates/syncthing-net/src/connection.rs`: 修复消息读取循环，正确实现 BEP 帧格式 `[4 bytes length][8 bytes header][payload]`
- `crates/syncthing-net/src/dialer.rs`: `DialConnector` trait 传递 TLS 配置，保持并行拨号与 TLS 兼容

**验证**:
- `cargo test -p syncthing-net` — **41 passed, 0 failed**（新增 `test_tls_hello_exchange`）

> **注意**: NET-TCP-FIX 子代理在编译调试过程中超时（900s），但代码修改已经成功落盘并验证通过。

### 3. CMD-DAEMON — `run` 子命令真正跑起来 ✅

**问题**: `cmd/syncthing run` 以前只启动了一个空 TCP listener，既不加载配置，也不启动 SyncService。

**修复内容**:
- `crates/syncthing-sync/src/service.rs`: 将 `start`/`stop` 方法改为 `pub`，新增 `pub async fn run()` 方法
- `crates/syncthing-net/src/manager.rs`: 签名改为接受外部 TLS 配置
- `cmd/syncthing/src/main.rs`:
  - 创建 `SyncService` + `MemoryDatabase`
  - 将同一个 `SyncthingTlsConfig` 传给 `ConnectionManager`
  - 连接/断开事件回调中调用 `sync_service.connect_device` / `disconnect_device`
  - 启动 `SyncService`，优雅关闭时先停 SyncService 再停 ConnectionManager
  - 新增集成测试 `test_daemon_start_stop`
  - 新增互操作测试代码块：自动读取 Go 端证书、添加 Go peer 和 test-folder、2 秒后自动拨号

**验证**:
- `cargo test -p syncthing` — **1 passed** (`test_daemon_start_stop`)
- `cargo build -p syncthing` — **成功**

### 4. BEP-MESSAGES — 扩展协议消息定义 ✅

**问题**: `bep-protocol` 只有 Hello 消息，缺少 `Request`/`Response`/`Index`/`ClusterConfig` 等核心消息。

**修复内容**:
- `crates/bep-protocol/src/messages.rs`:
  - 新增 10 个 `prost::Message` 派生结构体：`WireVector`, `WireCounter`, `WireBlockInfo`, `WireFileInfo`, `Request`, `Response`, `Index`, `IndexUpdate`, `ClusterConfig`, `WireFolder`
  - 新增 `encode_message` / `decode_message` 泛型辅助函数
  - 新增 `From` 转换：`FileInfo` ↔ `WireFileInfo`，`Vector` ↔ `WireVector`，`BlockInfo` ↔ `WireBlockInfo`
  - 新增 4 个单元测试
- `crates/bep-protocol/src/lib.rs`: 导出所有新类型和辅助函数

**验证**:
- `cargo test -p bep-protocol` — **17 passed, 0 failed**

### 5. SYNC-PULLER-NET — 网络块拉取对接 ✅ 完成

**问题**: `Puller::request_block` 返回全 0 字节，无法真正下载文件。

**修复内容**:
- `crates/syncthing-sync/src/puller.rs`:
  - 定义 `BlockSource` trait
  - `Puller` 新增 `block_source`，`with_block_source` builder 方法
  - `download_file` 从静态方法改为实例方法，通过 `block_source` 请求真实数据
  - 新增 `test_download_file_with_mock_source` 测试
- `crates/syncthing-sync/src/folder_model.rs`:
  - `FolderModel::new` 接受 `block_source` 并传给 `Puller`
- `crates/syncthing-sync/src/service.rs`:
  - `SyncService` 新增 `set_block_source`，支持在运行时注入块数据源
- `crates/syncthing-net/src/connection.rs`:
  - 为 `BepConnection` 实现 `recv_message() -> Result<(MessageType, Bytes)>`
  - 后台读取任务将解码后的消息推入内部 channel，供 `recv_message` 消费
- `cmd/syncthing/src/main.rs`:
  - 实现 `ManagerBlockSource`，通过 `ConnectionManagerHandle` 获取连接
  - 构造 BEP `Request` 消息，发送给任意已连接设备，等待匹配 `Response`
  - 在 `cmd_run` 中将 `ManagerBlockSource` 注入 `SyncService`
- `cmd/syncthing/Cargo.toml`:
  - 添加 `bep-protocol`、`bytes`、`async-trait` 依赖

**验证**:
- `cargo test -p syncthing-sync` — **28 passed, 0 failed**
- `cargo test -p syncthing-net` — **41 passed, 0 failed**
- `cargo build -p syncthing` — **成功**
- `cargo build --workspace` — **成功**

### 6. 互操作测试 — 与真实 Go Syncthing 握手 ✅ **重大突破**

**测试环境**:
- Go Syncthing 编译产物: `syncthing_go.exe`
- Go 端监听: `127.0.0.1:22001`
- Rust 端监听: `127.0.0.1:22000`
- 共享测试文件夹: `test-folder`

**测试中发现并修复的两个关键问题**:

#### (a) TLS Ed25519 签名算法缺失
- **现象**: Go 端拒绝握手，`tls: peer doesn't support any of the certificate's signature algorithms`
- **修复**: 在 `crates/syncthing-net/src/tls.rs` 的 `SyncthingClientCertVerifier` 和 `SyncthingCertVerifier` 中增加 `rustls::SignatureScheme::ED25519`

#### (b) Device ID Luhn-32 算法与 Go 不一致
- **现象**: Go 端校验 Rust 设备 ID 时报 `check digit incorrect`
- **修复**: 在 `crates/syncthing-core/src/device_id.rs` 中，将 `luhn32_char` 完全对齐到 Go 的参考实现（修正遍历方向，增加 `(addend / 32) + (addend % 32)` 步骤）

**互操作测试结果**:
- ✅ TCP 连接双向成功
- ✅ TLS 1.3 握手双向成功 (`TLS_AES_128_GCM_SHA256`)
- ✅ BEP Hello 交换双向成功
- ✅ 设备身份互相认证通过
- ✅ `ConnectionManager` 正确注册连接并触发上层回调

详细日志和过程记录在 `INTEROP_TEST_REPORT.md` 中。

---

## 三、今日 Git 提交

```
3576dd2 bep-protocol: add Request/Response/Index/ClusterConfig prost messages
e43898d mvp phase 1: fix TCP TLS + BEP frames, wire daemon startup
5019bb3 mvp phase 2: wire Puller to BEP Request/Response via BlockSource trait
```

加上互操作测试期间的两个关键修复（尚未提交）：
- `tls.rs`: 添加 Ed25519 支持
- `device_id.rs`: 修正 Luhn-32 算法

---

## 四、当前测试基线

```bash
cargo test -p syncthing-core -p syncthing-sync -p syncthing-net -p bep-protocol -p syncthing
```

| Crate | Passed | Failed | 备注 |
|-------|--------|--------|------|
| `syncthing-core` | 12 | 0 | 稳定，含 Luhn-32 测试 |
| `bep-protocol` | 17 | 0 | 新增 4 个 prost 测试 |
| `syncthing-net` | 41 | 0 | 新增 TLS Hello 测试 |
| `syncthing-sync` | 28 | 0 | 新增 mock block source 测试 |
| `syncthing` (cmd) | 1 | 0 | daemon 启动测试 |
| **合计** | **99** | **0** | — |

---

## 五、下一步待办

### 新增：BEP Index/ClusterConfig 互通完成（晚间追加）

在完成上述白天工作后，晚间继续推进互操作测试，成功打通了 BEP Index/ClusterConfig 的自动交换循环：

- **修复 TLS ALPN 协商**：在 `tokio_rustls` 客户端和服务端配置中显式添加 `bep/1.0` ALPN，消除了 Go 端的 `WRN Peer at did not negotiate bep/1.0` 警告。
- **修复 BEP 帧格式**：将 `BepConnection::send_message` / `recv_message` 从自定义的 `[magic][type][flags]` 8 字节头部，重写为标准 BEP 帧格式 `[2 bytes header_len][protobuf Header][4 bytes message_len][protobuf Message]`。
- **修复 ClusterConfig 设备信息**：在 `cmd/syncthing/src/tui/daemon_runner.rs` 中，为每个 `WireFolder` 的 `devices` 列表填充本地设备 ID、name 和 compression，满足 Go 端 `handling cluster-config: remote device missing in cluster config` 的校验要求。
- **互操作验证**：Rust 守护进程与 Go Syncthing 成功完成双向 ClusterConfig 交换，随后发送 Index 消息，连接进入 steady-state BEP 循环并保持稳定（30 秒以上测试通过，此前会在 1 秒内断开）。

---

## 五、下一步待办

1. **端到端文件同步验证**
   - 在 Go 端的 `test-folder` 放入一个真实文件
   - 验证 Rust 端的 Puller 能否通过网络完整拉取该文件

2. **推送 (Push) 方向**
   - 实现 `Request` 处理器，使 Rust 节点能响应远程块请求

3. **配置持久化**
   - 将 `cmd/syncthing run` 的硬编码配置迁移到 TOML/JSON 配置文件

4. **Web UI / 用户交互页面**
   - 当前项目**尚未实现** Web GUI 或 REST API
   - 如需交互界面，建议下一阶段启动 `syncthing-gui` 或 `syncthing-api` crate 的开发
