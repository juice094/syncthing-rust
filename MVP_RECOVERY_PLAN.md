# MVP 修复计划：让 syncthing-rust 真正跑起来

## 现状诊断（基于代码审计）

当前工作区有大量高质量的独立组件（TLS、扫描器、索引逻辑、监督树、iroh 隧道），但存在 4 个致命断裂点：

1. **TCP 传输没有 TLS** — `tcp_transport.rs` 创建裸 TCP 流后直接发送 BEP Hello，完全绕过了 `tls.rs` 中真实的 TLS 配置。
2. **TCP 路径发送 JSON 而非 protobuf Hello** — `protocol.rs` 中的 `HelloMessage::encode()` 输出 JSON 字符串，真实 Syncthing 无法解析。
3. **BEP 消息帧解析损坏** — `connection.rs` 中只跳过了 8 字节 header，没有处理 payload，导致读循环破坏流。
4. **cmd/syncthing run 是空壳 daemon** — 只启动了 TCP listener，没有加载配置、没有启动 SyncService、没有打通 sync ↔ net。
5. **Puller 只下载假数据** — `request_block` 返回全 0 字节，没有通过网络请求真实块。
6. **bep-protocol 只有 Hello** — 缺少 `Request`/`Response`/`Index`/`ClusterConfig` 等消息的编解码。

---

## 阶段划分

### Phase 1：网络层与 Daemon 启动（并行）✅ 已完成

#### Task 1: NET-TCP-FIX ✅
**目标**：修复 TCP 路径的 TLS + BEP Hello + 帧解析，使其能与真实 Syncthing 节点完成握手并保持连接。

**交付结果**：
- `tcp_transport.rs`：在 `connect_bep` / `listen` 中补上 `tokio_rustls` 客户端/服务端 TLS 握手 ✅
- `protocol.rs`：移除 JSON Hello，改为复用 `bep_protocol::handshake::Hello` 的 protobuf 编码 ✅
- `connection.rs`：修复 `BepConnection` 的消息读取循环，正确实现 `[4 bytes length][8 bytes header][payload]` 帧格式 ✅
- `manager.rs`：让 `ConnectionManager::new` 接受外部 `SyncthingTlsConfig` 而非重新生成证书 ✅
- 新增/更新测试：验证 TLS 握手成功、protobuf Hello 能被正确收发、帧解析不破坏流 ✅

#### Task 2: CMD-DAEMON ✅
**目标**：让 `cmd/syncthing run` 成为一个真正可运行的 daemon。

**交付结果**：
- `main.rs`：`run` 子命令创建 `SyncService` 和 `ConnectionManager` ✅
- 将同一个 `SyncthingTlsConfig` 传给网络层和同步层 ✅
- 启动 `SyncService::run()`（受 `Supervisor` 监督）✅
- 在 `ConnectionManager` 的回调中，将连接/断开事件路由到 `SyncService` ✅
- 优雅关闭时同时停止 `ConnectionManager` 和 `Supervisor` ✅
- 新增测试：daemon 启动后 `SyncService` 和 `ConnectionManager` 都处于运行状态 ✅

### Phase 2：BEP 消息协议与块拉取（串行或并行）✅ 已完成

#### Task 3: BEP-MESSAGES ✅
**目标**：扩展 `bep-protocol`，支持核心 BEP 消息的 prost 定义与编解码。

**交付结果**：
- 在 `bep-protocol/src/messages.rs` 中，用 `prost` 定义：
  - `Request` (id, folder, name, offset, size, hash, from_temporary) ✅
  - `Response` (id, data, error) ✅
  - `Index` / `IndexUpdate` (folder, files) ✅
  - `ClusterConfig` (folders, devices) ✅
- 提供编码/解码辅助函数 `encode_message` / `decode_message` ✅
- 新增测试：各消息 round-trip 正确 ✅

#### Task 4: SYNC-PULLER-NET ✅
**目标**：让 `Puller` 能通过网络请求真实块数据。

**交付结果**：
- 在 `syncthing-sync` 中定义 `BlockSource` trait ✅
- `Puller` 支持注入 `block_source`，`download_file` 调用真实请求 ✅
- `FolderModel` 和 `SyncService` 支持传递和设置 `BlockSource` ✅
- `BepConnection` 实现 `recv_message()` 供响应消费 ✅
- `cmd/syncthing/src/main.rs` 实现 `ManagerBlockSource`，构造 BEP `Request` 并等待 `Response` ✅
- 新增测试：mock 远程设备响应，验证块请求-响应流程 ✅

### Phase 3：互操作验证 ✅ 已完成

#### Task 5: INTEROP-TEST ✅
**目标**：与真实 Go Syncthing 进行端到端握手测试，修复兼容性问题。

**交付结果**：
- 编译并启动 Go Syncthing (`syncthing_go.exe`) ✅
- 运行 Rust 守护进程并自动拨号 Go 节点 ✅
- **发现并修复 TLS Ed25519 签名算法缺失** ✅
- **发现并修复 Device ID Luhn-32 算法与 Go 不一致** ✅
- 验证双向 TLS + BEP Hello 握手成功 ✅
- 编写 `INTEROP_TEST_REPORT.md` 记录全过程 ✅

---

## 验收标准

1. ✅ `cargo test -p syncthing-net --features iroh` 继续通过（40 passed）
2. ✅ `cargo test -p syncthing-core -p syncthing-sync -p syncthing-net -p bep-protocol` 全部通过
3. ✅ 修复后 `cmd/syncthing run` 能启动并保持运行至少 10 秒不崩溃
4. ✅ TCP 路径能与真实 Syncthing（或一个 mock TLS server）完成 TLS + protobuf Hello 交换
5. ✅ Puller 的 `request_block` 不再返回全 0

---

## 下一阶段建议（未开始）

### Phase 4：完整消息循环与端到端文件同步

1. **BEP-INDEX-LOOP**
   - 在 daemon 启动连接后，自动发送 `ClusterConfig` 和 `Index`
   - 接收并处理远程 `Index` / `IndexUpdate`，路由到 `FolderModel`

2. **END-TO-END-SYNC**
   - 在 Go 端文件夹放入测试文件
   - 验证 Rust 端能通过 Pull 完整下载并校验 SHA-256

3. **PUSH-SUPPORT**
   - 实现 BEP `Request` 处理器
   - 使 Rust 节点能向远程节点提供块数据

4. **CONFIG-PERSISTENCE**
   - 支持从 TOML/JSON 文件加载设备、文件夹配置
   - 替换 `cmd/syncthing run` 中的硬编码互操作测试块
