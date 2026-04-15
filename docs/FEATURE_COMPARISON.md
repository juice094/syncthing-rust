# syncthing-rust-rearch vs Go Syncthing 功能比对

**比对日期**: 2026-04-09  
**Go Syncthing 参考版本**: `dev/third_party/syncthing` (lib/model, lib/protocol, lib/connections)

---

## 一、总体评估

| 维度 | Go Syncthing | syncthing-rust-rearch | 差距 |
|------|--------------|----------------------|------|
| **核心同步能力** | 成熟生产级 | 骨架+逻辑层完成，网络对接进行中 | 中等 |
| **网络传输** | TCP+TLS / QUIC(relay) / NAT 穿透 | TCP+TLS 刚修复，iroh QUIC 可选 | 小~中等 |
| **协议完整性** | 完整 BEP v1.0 | Hello + Request/Response/Index 已定义，其余待验证 | 中等 |
| **配置与 UI** | Web UI + REST API + 完整配置系统 | CLI 仅 `run`/`generate-cert`/`show-id`，无 Web UI | 大 |
| **监控与可观测性** | 完整事件系统、审计日志 | 基础事件发布、tracing 日志 | 中等 |

---

## 二、按模块详细比对

### 2.1 身份与加密 (Identity & Crypto)

| 功能 | Go Syncthing | Rust 实现 | 状态 |
|------|--------------|-----------|------|
| DeviceId (Base32 + Luhn) | ✅ | ✅ | 完全对齐 |
| 自签名 TLS 证书 | ✅ | ✅ | 使用 `rcgen` |
| 证书 SHA-256 → DeviceId | ✅ | ✅ | 完全一致 |
| TLS 双向验证 (any cert) | ✅ | ✅ | 自定义 rustls verifier |
| Ed25519/QUIC 原生身份 | ❌ (使用 TLS) | ❌ (iroh 路径也做了 TLS-over-QUIC 隧道) | 一致 |

### 2.2 BEP 协议层 (Block Exchange Protocol)

| 消息类型 | Go Syncthing | Rust (`bep-protocol`) | 状态 |
|----------|--------------|----------------------|------|
| Hello (magic + length + protobuf) | ✅ | ✅ | 手写编解码完整 |
| Ping / Pong | ✅ | ✅ | 帧类型已定义，收发可用 |
| Close | ✅ | ✅ | 帧类型已定义 |
| Index / IndexUpdate | ✅ | ✅ | prost 定义已添加 |
| Request | ✅ | ✅ | prost 定义已添加 |
| Response | ✅ | ✅ | prost 定义已添加 |
| ClusterConfig | ✅ | ✅ | prost 定义已添加 |
| DownloadProgress | ✅ | ❌ | 未实现 |
| 消息帧定界 `[length][header][payload]` | ✅ | ✅ | 刚修复 |
| 压缩 (LZ4) | ✅ | ❌ | 未实现 |

### 2.3 网络连接层 (Connections)

| 功能 | Go Syncthing (`lib/connections`) | Rust (`syncthing-net`) | 状态 |
|------|-----------------------------------|------------------------|------|
| TCP + TLS 拨号 | ✅ | ✅ | 刚修复 |
| TCP + TLS 监听 | ✅ | ✅ | 刚修复 |
| 连接池管理 | ✅ | ✅ | `ConnectionManager` + `ConnectionEntry` |
| 地址发现 (local/global) | ✅ | ⚠️ | `DiscoveryManager` 存在但较简单 |
| 并行拨号 + 地址评分 | ✅ | ✅ | `ParallelDialer` 已实现 |
| 网络变更重绑定 | ✅ | ✅ | `NetMonitor` 已集成 |
| STUN (XOR-MAPPED-ADDRESS) | ✅ | ✅ | 从 Tailscale 移植 |
| UPnP / NAT-PMP 端口映射 | ✅ | ✅ | `PortMapper` 已实现 |
| PCP 端口映射 | ✅ | ❌ | `todo!()` stub |
| iroh/QUIC Relay | ✅ (实验性) | ✅ (可选 feature) | TLS-over-QUIC BEP 隧道 |
| 地址质量评分 (RTT/history) | ✅ | ✅ | `AddressScore` 已实现 |

### 2.4 同步引擎 (Sync Engine)

| 功能 | Go Syncthing (`lib/model`) | Rust (`syncthing-sync`) | 状态 |
|------|---------------------------|------------------------|------|
| 文件夹扫描 (walk + hash) | ✅ | ✅ | `Scanner` 完整 |
| 128KB 块哈希 (SHA-256) | ✅ | ✅ | 一致 |
| 版本向量 (Version Vector) | ✅ | ✅ |  dominance/conflict/merge |
| Delta Index (IndexID/Sequence) | ✅ | ✅ | 增量索引完整 |
| 冲突检测 | ✅ | ✅ | 逻辑完整 |
| 冲突文件创建 (.sync-conflict-) | ✅ | ✅ | 文件名格式对齐 |
| Puller (块拉取) | ✅ | 🔄 | `BlockSource` 已定义，对接中 |
| Pusher (块推送) | ✅ | ❌ | 未实现 |
| 临时文件 + 原子重命名 | ✅ | ✅ | `.syncthing.tmp` |
| 完整 ignore 规则 | ✅ | ⚠️ | 在独立工作区完成，未合并 |
| 监督树 / Supervisor | ✅ (`suture`) | ✅ | 自动重启 + 退避 |

### 2.5 数据库与持久化

| 功能 | Go Syncthing (`lib/db`) | Rust | 状态 |
|------|------------------------|------|------|
| LevelDB / Badger 元数据 | ✅ | ❌ | 当前只有内存/JSON DB |
| 文件缓存信息 | ✅ | ⚠️ | `MemoryDatabase` / `FileSystemDatabase` 有基础实现 |
| 块缓存 (block cache) | ✅ | ❌ | 未实现 |
| 配置持久化 (TOML/XML) | ✅ | ❌ | `run` 命令目前未加载配置文件 |

### 2.6 命令行与前端

| 功能 | Go Syncthing | Rust (`cmd/syncthing`) | 状态 |
|------|--------------|------------------------|------|
| Web GUI | ✅ | ❌ | 未开始 |
| REST API | ✅ | ❌ | 未开始 |
| 事件 API (/events) | ✅ | ❌ | 未开始 |
| CLI daemon (`run`) | ✅ | ✅ | 能启动 SyncService + ConnectionManager + BlockSource |
| CLI cert 管理 | ✅ | ✅ | `generate-cert` / `show-id` |

---

## 三、关键缺失项（阻碍生产使用）

1. **Push 方向**: 只能拉取，不能主动推送本地变更给远程节点。
2. **持久化配置**: 没有加载 `config.xml` / `config.toml`，每次启动都是默认空配置。
3. **完整数据库**: 缺少 LevelDB 等价物，大规模文件夹会性能不足。
4. **Web UI / REST API**: 完全没有，无法远程管理。
5. **真实互通验证**: TCP/TLS/BEP 修复后，尚未与 Go Syncthing 真实节点握手。
6. **ignore 规则集成**: `.stignore` 解析在另一代码库完成，未迁移到本工作区。

---

## 四、已有优势/特色

1. **现代化网络层**: 可选 `iroh` QUIC/Relay 传输，比 Go 版本的实验性 QUIC 路径更完整。
2. **监督树**: 使用 Rust `tokio` 实现的 `suture.Supervisor` 等价物，崩溃恢复明确。
3. **并行拨号器**: 地址评分 + 竞速拨号实现清晰，可扩展性强。
4. **TLS-over-QUIC BEP 隧道**: 巧妙绕过了 iroh 身份体系与 Syncthing DeviceId 的不兼容问题。
