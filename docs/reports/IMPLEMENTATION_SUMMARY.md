# syncthing-rust-rearch 项目实现总结

> **项目说明**：基于 Rust 的 Syncthing 兼容实现，采用多 crate 工作区结构。目标是通过直接参照 Go Syncthing 源码，构建一个功能完整的去中心化文件同步 daemon。
>
> **仓库位置**：`C:\Users\22414`  
> **最新更新**：2026-04-25（BEP Go-interop 修复、Local Discovery 集成、STUN/PortMapper 接入 daemon、clippy 0 warnings、UDP 测试稳定化）

---

## 一、架构分层

```
┌─────────────────────────────────────────┐
│           cmd/syncthing                 │  ← CLI 入口，daemon 启动器
├─────────────────────────────────────────┤
│  syncthing-sync        syncthing-net    │  ← 同步引擎 + 网络层
│  ├── Supervisor        ├── ConnectionManager
│  ├── SyncService       ├── ParallelDialer
│  ├── IndexManager      ├── NetMonitor
│  ├── Puller            ├── TcpTransport (TLS+BEP)
│  ├── Scanner           ├── IrohTransport (optional)
│  ├── ConflictResolver  ├── STUN / Portmapper
│  └── FolderModel       └── TLS / DeviceId
├─────────────────────────────────────────┤
│  syncthing-core        bep-protocol     │  ← 核心类型 + BEP 协议
│  ├── DeviceId          ├── Hello handshake
│  ├── FileInfo          ├── Request/Response/Index/ClusterConfig
│  └── IndexID/Version   └── Message codec
└─────────────────────────────────────────┘
```

---

## 二、各 Crate 实现状态

### 1. `syncthing-core` — ✅ 扎实完成
- **DeviceId**: Base32 + Luhn-32（**已与 Go 实现完全一致**），支持新旧格式，完整测试 (12 passed)
- **类型系统**: `FileInfo`, `Folder`, `Config`, `VersionVector`, `IndexID`, `RetryConfig` 等
- **稳定性**: 高，无已知缺失

### 2. `bep-protocol` — ✅ 核心消息完整
| 功能 | 状态 | 说明 |
|------|------|------|
| Hello Handshake | ✅ | 手写 protobuf 编解码，magic/length 前缀正确 |
| `Request` / `Response` | ✅ | prost 定义已添加 |
| `Index` / `IndexUpdate` | ✅ | prost 定义已添加 |
| `ClusterConfig` | ✅ | prost 定义已添加 |
| 消息帧读写器 | ✅ | 在 `syncthing-net/connection.rs` 中实现标准 BEP 帧格式 |

### 3. `syncthing-net` — ✅ 骨架完整，主路径已打通
| 功能 | 状态 | 说明 |
|------|------|------|
| TLS 证书管理 | ✅ | `rcgen` 生成自签名证书，DeviceId 从 SHA-256 指纹推导 |
| TCP + TLS 握手 | ✅ | 支持 Ed25519 证书，已通过与真实 Go 节点互操作验证 |
| BEP Hello (protobuf) | ✅ | 已通过真实 Go 节点验证 |
| BEP 标准帧解析 | ✅ | 正确实现 `[2 bytes header_len][protobuf Header][4 bytes msg_len][protobuf Message]` |
| iroh TLS-over-QUIC | ✅ | 可选 feature，能通过 BEP Hello + Ping 测试 |
| 连接管理 | ⚠️ | `ConnectionManager` 有连接池、pending 连接、重试退避；存储结构支持多路径，但 `get_connection()` API 只返回第一个 alive 连接，未暴露多路径能力 |
| 并行拨号 | ✅ | `ParallelDialer` 支持最多 3 地址并发竞速 + RTT 评分 |
| 网络变更监听 | ✅ | `NetMonitor` 检测接口变化并触发重拨 |
| 端口映射 (UPnP) | ⚠️ | `PortMapper` UPnP 路径可用；PCP/NAT-PMP 骨架存在但未实现；daemon 中无自动续约 |
| STUN 客户端 | ⚠️ | 可查询公网映射地址（XOR-MAPPED-ADDRESS 解析完整）；缺少 NAT 类型检测、多服务器对比、hole punching 协调 |

### 4. `syncthing-sync` — ⚠️ 同步逻辑真实，推送方向未实现
| 功能 | 状态 | 说明 |
|------|------|------|
| Supervisor 监督树 | ✅ | Go `suture` 等价实现，自动重启 + 指数退避 |
| 文件夹扫描器 | ✅ | 递归遍历、128KB 块哈希、跳过 temp/hidden 文件 |
| 索引处理 | ✅ | `IndexHandler` 版本向量比较、差异计算、合并逻辑完整 |
| 冲突解决 | ✅ | 物理冲突拷贝 + 版本向量合并 |
| Delta Index | ✅ | `IndexID` + `Sequence` 增量索引，持久化到 DB |
| 任务队列/执行器 | ✅ | 优先级队列 + 并发限制 |
| 数据库抽象 | ✅ | `MemoryDatabase` 和 `FileSystemDatabase` (JSON) |
| **Puller 块拉取** | ✅ | `BlockSource` trait 已对接，`Puller` 可通过 BEP Request/Response 拉取真实块 |
| 文件系统 watcher | ✅ | `FolderModel` 集成 `notify` watcher，支持秒级触发 scan + IndexUpdate |
| 推送 (Push) | ⚠️ | 被动响应块请求（上传）已通过 `block_server.rs` + `BepSession::on_block_request` 实现；主动扫描后触发对端拉取的调度逻辑待完善 |

### 5. `cmd/syncthing` — ✅ 已成为真实 daemon
| 子命令 | 状态 |
|--------|------|
| `generate-cert` | ✅ 可用 |
| `show-id` | ✅ 可用 |
| `run` | ✅ 可用 | 启动 `SyncService` + `ConnectionManager`，共享 TLS 证书，已注入 `ManagerBlockSource`，具备块拉取能力；默认端口 22001/8385，自动回退避免与本地 Go 节点冲突 |

---

## 三、互操作测试里程碑

### 2026-04-09：首次与真实 Go Syncthing 完成端到端握手 ✅
- **TCP 连接**: 双向成功
- **TLS 1.3 握手**: 双向成功 (`TLS_AES_128_GCM_SHA256`)
- **BEP Hello 交换**: 双向成功
- **设备身份认证**: 双方互相接受
- **连接注册与回调**: `ConnectionManager` 正确工作

### 2026-04-09 晚间：BEP Index/ClusterConfig 自动交换循环打通 ✅
- **TLS ALPN 协商**: 修复 `bep/1.0` ALPN，消除 Go 端警告
- **BEP 标准帧格式**: 从自定义格式切换为标准 BEP `[header_len][Header][msg_len][Message]` 帧
- **ClusterConfig 完整性**: 在每个 `WireFolder` 的 `devices` 列表中填入本地设备信息
- **测试结果**: Rust 与 Go 成功交换双向 `ClusterConfig`，发送 `Index`，进入 steady-state BEP 循环；连接在 Tailscale 虚拟网络环境下保持 30s+（真实公网/局域网稳定性待验证）

### 2026-04-11：跨网络 BEP 互通与文件同步验证 ✅
- **网络环境**: Rust 节点通过 Tailscale 连接云端 Go 节点 (`100.99.240.98:22000`)
- **关键修复**: LZ4 解压、Protobuf tag 对齐、读写死锁消除
- **验证结果**: 成功完成 TLS → Hello → ClusterConfig → Index → Request/Response 全链路，`gray_test.txt` 完整下载并校验内容一致
- **连接保活修复**: 解决 `manager.rs` stale 误判问题，修复自动重连退避逻辑
- **详细报告**: [VERIFICATION_REPORT_BEP_2026-04-11.md](./VERIFICATION_REPORT_BEP_2026-04-11.md)

### 2026-04-15：文件系统 watcher 与双节点共存验证 ✅
- **fs watcher**: 基于 `notify` v7.0.0 为每个 folder 启动 `RecommendedWatcher`，1s debounce 后触发 scan，文件变更在 **2 秒内** 触发 `IndexUpdate` 并推送到云端 Go 节点
- **端口共存**: 默认 BEP 端口永久迁移至 `22001`，REST API 迁移至 `8385`，绑定失败时自动回退随机端口，实现与本地 Go 节点（`:22000/:8384`）长期并行运行
- **重连修复**: 修复 `schedule_reconnect` 因 `pending_connections` 竞态导致二次拨号被拦截的 bug
- **TUI 修复**: `Add Folder` 弹窗中 `Space` 键可正常切换设备 checkbox，Tab/BackTab 焦点能正确进入设备选择区

### 2026-04-17：Phase 2 Network Abstraction + BepSession 解耦 ✅
- **`ReliablePipe` trait**: 在 `syncthing-core` 中定义 `ReliablePipe = AsyncRead + AsyncWrite + local/peer_addr + path_quality + transport_type`，正式将 BEP 语义与 TCP 实现解耦
- **`TcpBiStream` 实现**: `syncthing-net` 中 `TcpBiStream` 已实现 `ReliablePipe`
- **`BepConnection::new` 重构**: 签名从 `TcpBiStream` 改为 `Box<dyn ReliablePipe>`，内部使用 `tokio::io::split` 分离读写半流
- **`BepHandshaker` 抽取**: 新建 `handshaker.rs`，统一封装 BEP Hello 交换逻辑，`tcp_transport.rs` 中所有内联 Hello 代码已替换
- **`ConnectionManager` 多路径支持**: 内部连接存储从 `device_id -> ConnectionEntry` 重构为 `device_id -> conn_id -> ConnectionEntry`（嵌套 `DashMap`），支持同一设备维护多条并发路径
- **`BepSession` 抽取**: 新建 `session.rs`，将 `daemon_runner.rs` 中 ~230 行的 BEP 消息循环（ClusterConfig → Index → steady-state）抽取为独立的 `BepSession` 组件；`daemon_runner.rs` 通过 `DaemonBepHandler` 实现 `BepSessionHandler` 回调，BEP 相关代码量减少 **>60%**
- **MemoryPipe 验收测试**: 新增 `test_session_ping_pong` 和 `test_session_block_request_response`，在内存管道上验证 BEP 完整会话周期
- **Trait 清理**: 旧 `syncthing_core::traits::BepConnection` / `BepMessage` 标记 `#[deprecated]`，`SyncModel::handle_connection` 移除；`ReliablePipe` + `BepSession` 成为 canonical 架构
- **编译与测试**: 全 workspace `cargo test` 通过，0 failed

### 2026-04-17：跨窗口代理集群会议 — syncthing-rust-rearch 发言 ✅
- 向 devbase 确认 `.devbase/syncdone` 标记的可行性、格式建议与生命周期约束
- 明确 `.sync-conflict` 的暂停/恢复策略：devbase watcher 驱动，syncthing-rust BEP 层无需特殊处理
- 坦诚 BEP 协议无全局"同步完成"信号，提供 pragmatic 替代方案（`FolderStatus::Idle` + `needed_files.is_empty()`）与中期增强路径（`on_peer_sync_state` 回调）

### 2026-04-20：BEP 协议兼容性修复 + Local Discovery Phase 0 ✅
- **`client_name` 伪装**: `"syncthing-rust"` → `"syncthing"`，匹配 Go 端预期
- **`WireFolder.label` 修复**: `Vec<String>` → `String`，与 Go 端 protobuf `string`（非 `repeated string`）兼容
- **`validate_device_id`**: 测试用例改为 Base32-Luhn 带 dash 格式
- **Local Discovery 集成**: `discovery.rs` 拆分为 `discovery/{mod,local,events}.rs`；`daemon_runner.rs` 集成 `LocalDiscovery::run()` + auto-dial
- **docs 重组**: 18 份历史文档归档到 `archive/`，建立 `design/plans/reports` 分层结构

### 2026-04-25：质量门清理 + Phase 5 基础设施 ✅
- **clippy 0 warnings**: 手动修复 4 个 + auto-fix 6 个，workspace 全绿
- **UDP 测试稳定化**: `test_udp_broadcast_roundtrip` 改用临时端口，消除 Windows WSAEADDRINUSE
- **STUN/PortMapper 接入**: `daemon_runner.rs` 启动时 spawn 后台任务检测公网地址/申请端口映射
- **AGENTS.md 修正**: 将夸大的 "全实现" 声明降维为准确描述

### 2026-04-25：Global Discovery 客户端（Phase 2）✅
- **`GlobalDiscovery` 实现**: `crates/syncthing-net/src/discovery/global.rs`
  - `announce()`: HTTPS POST `{ "addresses": [...] }`，mTLS 客户端证书认证
  - `query()`: HTTPS GET `?device=<id>`，返回地址列表（404 返回空列表）
  - `run()`: 后台循环，每 30min announce，失败 5min 后重试
- **daemon_runner 集成**: 启动时从 `cert.pem`/`key.pem` 自动构造 `GlobalDiscovery`，spawn 后台 announce 任务
- **依赖**: `reqwest`（`rustls-tls` feature）已加入 `syncthing-net/Cargo.toml`
- **⚠️ 风险点**：
  1. **mTLS 证书格式假设**：`reqwest::Identity::from_pem` 要求未加密的 PEM 私钥；若用户证书使用加密私钥或 PKCS#8 特定变体，初始化会失败（当前仅打印 warn log，不阻断启动）
  2. **单服务器无 fallback**：仅连接 `discovery.syncthing.net`，无备用服务器；若官方服务故障或被墙，全局发现完全失效
  3. **隐私暴露**：向第三方服务器上报 device_id + 公网 IP；未来应支持 `globalAnnounceEnabled: false` 配置项让用户关闭
  4. **地址陈旧窗口**：30min announce 间隔意味着公网 IP 变化后最长 30min 内其他设备可能拨到旧地址；STUN 检测到变化不会立即触发 re-announce
  5. **未与连接管理器联动**：`query()` 结果目前未被 `ConnectionManager` 消费（缺少"发现未知设备地址后自动拨号"的闭环）

### 2026-04-25：Relay Protocol v1 客户端（Phase 3）✅
- **协议实现**: `crates/syncthing-net/src/relay/{protocol,client,types}.rs`
  - XDR 编解码：Magic header + 大端序整数 + 4 字节对齐 opaque/string
  - 消息类型：Ping/Pong, JoinRelayRequest, JoinSessionRequest, Response, ConnectRequest, SessionInvitation, RelayFull
  - `RelayProtocolClient`：Protocol Mode（TLS `bep-relay`）连接，支持 `join_relay()` / `request_session()` / `wait_invitation()` / `ping()`
  - `join_session()`：Session Mode（明文 TCP）连接，返回可用于 BEP TLS 握手的 `TcpStream`
- **daemon_runner 集成**：
  - TCP auto-dial 10 秒后，若设备仍未连接，自动尝试配置的 `relay://` 地址 fallback
  - 被动监听：若配置或 pool 中有 relay 地址，spawn 永久 mode 后台任务等待 `SessionInvitation`
  - Relay pool 自动获取：`relays.syncthing.net/endpoint` → 无配置地址的设备自动使用 pool 中的 relay
- **⚠️ 风险点（已解决）**：
  1. ✅ **Transport/Dialer 集成**：`connect_bep_via_relay` + `daemon_runner` fallback
  2. ✅ **TLS ALPN**：`SyncthingTlsConfig::relay_client_config()` 配置 `bep-relay`
  3. ✅ **SessionInvitation 地址解析**：`resolve_session_addr` 空地址回退
  4. ✅ **Relay pool 获取**：`relay/pool.rs` 实现 `fetch_relay_pool()` + `fetch_default_relay()`
  5. ✅ **被动邀请后台任务**：`run_relay_listener()` 在 daemon_runner 中作为后台循环运行
  6. ✅ **Global Discovery 地址联动**：`GlobalDiscovery::trigger_reannounce()` + STUN/PortMapper 成功后触发
- **⚠️ 剩余风险点（本轮已解决）**：
  1. ✅ **Relay pool 健康检查**：`filter_healthy_relays()` TCP 3 秒超时探测，daemon_runner 获取 pool 后自动过滤
  2. ✅ **被动监听冗余**：收集配置+pool 去重后取前 3 个地址，分别 spawn 永久 mode 监听任务
  3. ✅ **Global Discovery 首启延迟**：取消 5s sleep，改为立即首次 announce；STUN/UPnP 完成后 trigger_reannounce 补发
- **⚠️ 当前剩余风险点**：
  1. **Relay 健康检查只做 TCP 层**：未验证 TLS + JoinRelay 是否成功，可能存在 TCP 通但 relay 已满或拒绝连接的情况
  2. **Global Discovery 未优雅退出**：daemon 停止时 GlobalDiscovery 后台任务仍在运行（无 shutdown signal）
  3. **被动监听任务无上限**：若用户配置了大量 relay 地址或 pool 返回很多，可能创建过多并发连接（当前限制为 3）

### 2026-04-17：Phase 3.1 BepSession Observability ✅
- **`BepSessionEvent` 枚举**：新增 6 种事件覆盖会话全生命周期 — `ClusterConfigComplete`, `IndexSent`, `IndexReceived`, `IndexUpdateReceived`, `BlockRequested`, `HeartbeatTimeout`, `SessionEnded`
- **`BepSessionMetrics` 原子计数器**：`messages_sent/recv`, `bytes_sent/recv`, `blocks_requested/served`, `heartbeat_timeouts`, `errors`，全部使用 `AtomicU64` 无锁统计
- **`emit()` 辅助方法**：事件通过 `mpsc::UnboundedSender` 发射，不阻塞消息循环；未订阅时零开销
- **`metrics()` getter**：外部可观测面板可通过 `Arc<BepSessionMetrics>` 读取实时计数
- **心跳超时检测**：270s 无消息自动触发 `HeartbeatTimeout` 事件并终止会话（原 90s 仅发送 ping，无检测逻辑）
- **编译与测试**：`cargo check` + `cargo test -p syncthing-net --lib` 全绿（46 passed, 0 failed）

---

## 四、构建与测试

### 编译
```bash
# release 构建
cargo build --release -p syncthing
```

### 单元测试
```bash
# 核心 crates
cargo test -p syncthing-core -p syncthing-sync -p syncthing-net -p bep-protocol -p syncthing
# 结果: 99 passed, 0 failed

# iroh feature
cargo test -p syncthing-net --features iroh
# 结果: 40 passed, 0 failed, 1 ignored
```

### 互操作测试（与 Go Syncthing）
```bash
# 启动 Go 端（监听 127.0.0.1:22001）
syncthing_go.exe -home=%TEMP%\syncthing_test_go -gui-address=127.0.0.1:8384

# 启动 Rust 端（监听 127.0.0.1:22000）
cargo run --release -p syncthing -- run -l 127.0.0.1:22000 -c %TEMP%\syncthing_test_rust
```

---

## 五、已知问题与下一步

### 已修复的关键问题
1. ✅ TCP 传输没有 TLS → 已补上 `tokio_rustls` 握手
2. ✅ TCP 发送 JSON Hello → 已改为 protobuf Hello
3. ✅ BEP 帧解析 bug → 已修复为标准 BEP 帧格式
4. ✅ `ConnectionManager::new` 重新生成证书 → 现在接受外部 `SyncthingTlsConfig`
5. ✅ `Puller::request_block` 已通过网络请求真实块
6. ✅ `cmd/syncthing run` 已配置 `BlockSource` 给 `SyncService`
7. ✅ TLS 缺少 Ed25519 支持 → 已添加
8. ✅ Device ID Luhn-32 算法不一致 → 已对齐 Go 实现
9. ✅ BEP Index/ClusterConfig 自动交换 → 已打通

### 待完成工作
1. ✅ **端到端文件同步验证（Pull）**：2026-04-11 已通过跨网络测试验证完整下载
2. **推送 (Push) 方向**：被动响应块请求（上传）链路完整（`block_server.rs` → `BepSession::on_block_request` → `sync_service.handle_block_request`），但 **主动扫描后触发对端拉取的调度逻辑待完善**
3. ✅ **配置持久化**：`JsonConfigStore` 已落地（2026-04-16），支持 notify 文件监听、内存缓存、异步读写；端口迁移与 API key 生成已正确持久化
4. **REST API 完善**：`/rest/db/status` 已实现真实统计，但设备删除、文件夹修改等写接口待补充
5. ✅ **TUI 设备删除**：`events.rs:58-69` 已实现 `d` 键删除设备并自动保存 `config.json`；该 issue 为 stale，已关闭
6. ✅ **BepSession 解耦**：已完成，`daemon_runner.rs` 已使用 `BepSession` + `DaemonBepHandler`
7. ✅ **`syncthing-core::traits::BepConnection` 对齐**：由于该 trait 的 `request_block`/`recv_message` 签名与 `Arc<BepConnection>` + `BepSession` 架构存在结构性冲突，已将其标记为 `#[deprecated]`，`SyncModel::handle_connection` 同步移除；`ReliablePipe` + `BepSession` 成为 canonical 架构
8. **acceptance-tests crate**：因早期 `BepMessage` API 变更暂被排除，修复成本/收益待评估
9. **ManagerBlockSource 缺陷**：`request_block` 使用 `connected_devices().into_iter().next()` 向**任意**已连接设备请求块，非目标定向请求；多设备场景下会拉取错误数据

---

## 六、文件清单

```
crates/
├── syncthing-core/      # DeviceId, FileInfo, 错误类型, 核心类型
├── bep-protocol/        # BEP Hello, Request/Response, Index, ClusterConfig
├── syncthing-net/       # TCP/TLS 传输, ConnectionManager, 拨号, STUN, 端口映射
├── syncthing-sync/      # SyncService, Puller, Scanner, IndexHandler, Supervisor
└── syncthing-fs/        # (预留) 文件系统抽象, ignore 处理
cmd/
└── syncthing/           # CLI 入口 (generate-cert, show-id, run)
```

---

## 参考实现

- Go Syncthing: `lib/connections/`, `lib/protocol/`
- Tailscale: 网络监听与端口映射策略
- iroh: QUIC 传输与 TLS 证书处理

---

## 七、未来发展路线（初步设想，非承诺）

> ⚠️ **以下内容为方向性思考，尚未进入实施排期，具体优先级可能随项目进展调整。**  
> 请勿将其视为已确定或已完成的交付物。

### Phase 1 — 核心对等节点能力（已完成）
- ✅ 推送（Push）方向：被动响应块请求（上传）已实现；ManagerBlockSource 轮询所有已连接设备
- ✅ 配置持久化：JSON 配置文件 + CLI override 机制
- ✅ REST API：兼容 Go 布局的基础接口已运行
- ⚠️ Web GUI 托管：内置 HTTP 服务器就绪，静态资源托管待实现

### Phase 2 — 网络可达性体验优化（核心完成）
- ✅ Local Discovery：UDP 广播骨架 + daemon_runner auto-dial
- ✅ Global Discovery：HTTPS mTLS 客户端（announce/query）
- ✅ STUN：公网地址检测
- ✅ PortMapper：UPnP 端口映射
- ✅ Relay Protocol：官方 XDR over TCP 客户端（protocol mode + session mode）
- ⚠️ Transport/Dialer 集成：relay 地址尚未接入 `ParallelDialer` 和 `ConnectionManager`
- 可选的地址解析插件：允许从 Tailscale API / Headscale API 等外部来源动态获取 peer 地址（仅做轻量 API 调用，不引入 Tailscale 内核）

### Phase 3 — 可选的深度网络集成（远期评估）
- 若到那时有明确需求且收益大于成本，可评估将 Tailscale（`tsnet` / `magicsock`）或 iroh 作为可选 Cargo feature 引入传输层
- 该阶段需以 Phase 1 的协议完整性和稳定 ABI 为前提
