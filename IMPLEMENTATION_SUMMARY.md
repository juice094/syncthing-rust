# syncthing-rust-rearch 项目实现总结

> **项目说明**：基于 Rust 的 Syncthing 兼容实现，采用多 crate 工作区结构。目标是通过直接参照 Go Syncthing 源码，构建一个功能完整的去中心化文件同步 daemon。
>
> **仓库位置**：`C:\Users\22414`  
> **最新更新**：2026-04-09

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
| 连接管理 | ✅ | `ConnectionManager` 有连接池、pending 连接、重试退避 |
| 并行拨号 | ✅ | `ParallelDialer` 支持最多 3 地址并发竞速 + RTT 评分 |
| 网络变更监听 | ✅ | `NetMonitor` 检测接口变化并触发重拨 |
| 端口映射 (UPnP/NAT-PMP) | ✅ | `PortMapper` 已实现 |
| STUN 客户端 | ✅ | XOR-MAPPED-ADDRESS 解析完整 |

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
| 推送 (Push) | ❌ | 未实现，只有拉取 (Pull) 方向 |

### 5. `cmd/syncthing` — ✅ 已成为真实 daemon
| 子命令 | 状态 |
|--------|------|
| `generate-cert` | ✅ 可用 |
| `show-id` | ✅ 可用 |
| `run` | ✅ 可用 | 启动 `SyncService` + `ConnectionManager`，共享 TLS 证书，已注入 `ManagerBlockSource`，具备块拉取能力 |

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
- **测试结果**: Rust 与 Go 成功交换双向 `ClusterConfig`，发送 `Index`，进入**稳定 steady-state BEP 循环**，连接持续保持（30s+ 测试通过）

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
1. **端到端文件同步验证**：配置真实测试文件，验证 Pull 方向能否完整下载
2. **推送 (Push) 方向**：实现块的上传响应能力
3. **配置持久化**：将 `run` 命令的硬编码配置迁移到 TOML/JSON 配置文件
4. **Web UI / REST API**：当前尚未实现 Web GUI 或 REST API

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
