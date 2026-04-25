# syncthing-rust-rearch 项目状态报告

**报告日期**: 2026-04-09  
**仓库位置**: `C:\Users\22414` (Git 已初始化)  
**最新提交**: `待本次更新后提交`

---

## 一、项目简介

`syncthing-rust-rearch` 是一个基于 Rust 的 Syncthing 兼容实现项目，采用多 crate 工作区结构。项目目标是通过直接参照 Go Syncthing 源码以及 Tailscale/iroh 等第三方项目，构建一个功能完整的去中心化文件同步 daemon。

### 架构分层

```
┌─────────────────────────────────────────┐
│           cmd/syncthing                 │  ← CLI 入口，daemon 启动器
├─────────────────────────────────────────┤
│  syncthing-sync        syncthing-net    │  ← 同步引擎 + 网络层
│  ├── Supervisor        ├── ConnectionManager
│  ├── SyncService       ├── ParallelDialer
│  ├── IndexManager      ├── NetMonitor
│  ├── Puller            ├── TcpTransport (TLS+BEP)
│  ├── Scanner           ├── IrohTransport (*)
│  ├── ConflictResolver  ├── STUN / Portmapper
│  └── FolderModel       └── TLS/DeviceId
├─────────────────────────────────────────┤
│  syncthing-core        bep-protocol     │  ← 核心类型 + BEP 协议
│  ├── DeviceId          ├── Hello handshake
│  ├── FileInfo          ├── Request/Response/Index
│  └── IndexID/Version   └── Message codec
└─────────────────────────────────────────┘
        (*) optional feature `iroh`
```

---

## 二、各 Crate 真实状态

### 1. `syncthing-core` — ✅ 扎实完成
- **DeviceId**: Base32 + Luhn-32（**已与 Go 实现完全一致**），支持新旧格式，完整测试 (12 passed)
- **类型系统**: `FileInfo`, `Folder`, `Config`, `VersionVector`, `IndexID`, `RetryConfig` 等
- **稳定性**: 高，无已知缺失

### 2. `bep-protocol` — ⚠️ 部分完成
| 功能 | 状态 | 说明 |
|------|------|------|
| Hello Handshake | ✅ | 手写 protobuf 编解码，magic/length 前缀正确 |
| `Request` / `Response` | ✅ | prost 定义已添加 |
| `Index` / `IndexUpdate` | ✅ | prost 定义已添加 |
| `ClusterConfig` | ✅ | prost 定义已添加 |
| 消息帧读写器 | ✅ | 在 `syncthing-net/connection.rs` 中实现，互操作测试验证通过 |

### 3. `syncthing-net` — ✅ 骨架完整，主路径已打通
| 功能 | 状态 | 说明 |
|------|------|------|
| TLS 证书管理 | ✅ | `rcgen` 生成自签名证书，DeviceId 从 SHA-256 指纹推导 |
| TCP + TLS 握手 | ✅ | **已修复**（MVP Phase 1），支持 Ed25519 证书 |
| BEP Hello (protobuf) | ✅ | **已修复**，已通过真实 Go 节点验证 |
| BEP 帧解析 | ✅ | **已修复**，`connection.rs` 正确解析 `[4 bytes length][8 bytes header][payload]` |
| iroh TLS-over-QUIC | ✅ | 可选 feature，能通过 BEP Hello + Ping/Pong 测试 |
| 连接管理 | ✅ | `ConnectionManager` 有连接池、pending 连接、重试退避 |
| 并行拨号 | ✅ | `ParallelDialer` 支持最多 3 地址并发竞速 + RTT 评分 |
| 网络变更监听 | ✅ | `NetMonitor` 检测接口变化并触发重拨 |
| 端口映射 (UPnP/NAT-PMP) | ✅ | `PortMapper` 已实现，PCP 为 stub |
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
| **Puller 块拉取** | ✅ | `BlockSource` trait 已定义，`ManagerBlockSource` 在 `cmd/syncthing` 中已对接，`Puller` 可通过 BEP Request/Response 拉取真实块 |
| 推送 (Push) | ❌ | 未实现，只有拉取 (Pull) 方向 |

### 5. `cmd/syncthing` — ✅ 已成为真实 daemon
| 子命令 | 状态 |
|--------|------|
| `generate-cert` | ✅ 可用 |
| `show-id` | ✅ 可用 |
| `run` | ✅ 可用 | 启动 `SyncService` + `ConnectionManager`，共享 TLS 证书，已注入 `ManagerBlockSource`，具备块拉取能力 |

---

## 三、互操作测试里程碑 ✅

**2026-04-09 晚间，Rust 守护进程首次与真实 Go Syncthing 完成端到端握手。**

详细报告见 `INTEROP_TEST_REPORT.md`。

### 测试结论
- **TCP 连接**: 双向成功
- **TLS 1.3 握手**: 双向成功 (`TLS_AES_128_GCM_SHA256`)
- **BEP Hello 交换**: 双向成功
- **设备身份认证**: 双方互相接受
- **连接注册与回调**: `ConnectionManager` 正确工作

---

## 四、已知关键问题与风险

### 已修复
1. ✅ TCP 传输没有 TLS → 已补上 `tokio_rustls` 握手
2. ✅ TCP 发送 JSON Hello → 已改为 protobuf Hello
3. ✅ BEP 帧解析 bug → 已修复 `[length][header][payload]` 解析
4. ✅ `ConnectionManager::new` 重新生成证书 → 现在接受外部 `SyncthingTlsConfig`
5. ✅ `Puller::request_block` 已通过网络请求真实块
6. ✅ `cmd/syncthing run` 已配置 `BlockSource` 给 `SyncService`
7. ✅ TLS 缺少 Ed25519 支持 → 已添加
8. ✅ Device ID Luhn-32 算法不一致 → 已对齐 Go 实现

### 仍在进行中
1. 🔄 **没有推送实现**（只能下载，不能上传）
2. ❌ 没有 `.stignore` 集成到当前工作区（`syncthing-fs` crate 缺失）
3. ❌ 没有持久化配置加载（`run` 命令目前使用硬编码或默认配置）
4. ✅ **BEP Index/ClusterConfig 消息循环已在 daemon 中打通**
   - Rust 与 Go 成功交换双向 ClusterConfig 并发送 Index
   - 连接进入 steady-state，可稳定保持

---

## 五、测试基线

```bash
# 默认 feature（核心 crates）
cargo test -p syncthing-core -p syncthing-sync -p syncthing-net -p bep-protocol -p syncthing
# 结果: 99 passed, 0 failed

# iroh feature
cargo test -p syncthing-net --features iroh
# 结果: 40 passed, 0 failed, 1 ignored

# CLI
cargo test -p syncthing
# 结果: 1 passed (test_daemon_start_stop)

# 构建
cargo build -p syncthing
# 结果: 成功
```

---

## 六、Git 提交历史

```
5019bb3 mvp phase 2: wire Puller to BEP Request/Response via BlockSource trait
3d4b667 docs: add project status, feature comparison, and today work report
3576dd2 bep-protocol: add Request/Response/Index/ClusterConfig prost messages
e43898d mvp phase 1: fix TCP TLS + BEP frames, wire daemon startup
1e2999a iroh: TLS-over-QUIC BEP tunnel with DeviceId from peer certs
db73d8f wave3 milestone: net rebind, supervisor, parallel dialer
653daea add wave3 execution plan: net rebind, parallel dialer, supervisor
ee4bf02 wave2 milestone: delta index, conflict resolution, workspace compile fixes
```

---

## 七、下一步建议

1. **端到端文件同步**: 配置一个真实测试文件，验证 Pull 方向能否完整下载
2. **推送 (Push) 方向**: 实现块的上传响应能力
3. **配置持久化**: 将 `run` 命令的硬编码配置迁移到 TOML/JSON 配置文件
