# Syncthing Rust vs Go 原版 - 兼容性与流程验证比较

**分析日期**: 2026-04-04  
**Rust 版本**: syncthing-rust-rearch (实验性)  
**Go 版本**: syncthing-main (官方原版)

---

## 一、架构对比

### 1.1 模块结构对比

| 功能领域 | Go 原版 (lib/) | Rust 版本 (crates/) | 状态 |
|----------|----------------|---------------------|------|
| 核心类型 | `protocol/` | `syncthing-core` | ⚠️ 部分兼容 |
| 文件系统 | `fs/`, `scanner/`, `ignore/` | `syncthing-fs` | ⚠️ API差异 |
| 数据库 | `db/`, `internal/db/sqlite/` | `syncthing-db` | ❌ 不兼容 |
| 网络层 | `connections/`, `discover/`, `relay/` | `syncthing-net` | ❌ 完全不同 |
| 协议实现 | `protocol/` | `bep-protocol` | ⚠️ 格式差异 |
| 同步模型 | `model/` | `syncthing-sync` | ⚠️ 骨架实现 |
| API层 | `api/`, `gui/` | `syncthing-api` | ⚠️ 部分兼容 |
| 配置管理 | `config/` | `syncthing-api/config` | ⚠️ 格式差异 |

### 1.2 关键架构差异

```
Go 原版架构:
┌─────────────────────────────────────────────────────────────┐
│                      lib/api (REST)                         │
├─────────────────────────────────────────────────────────────┤
│                      lib/model (Sync)                       │
├─────────────────────────────────────────────────────────────┤
│  lib/protocol  │  lib/connections  │  lib/discover         │
│  (BEP协议)      │  (TCP/TLS连接)     │  (本地+全球发现)       │
├─────────────────────────────────────────────────────────────┤
│  lib/fs        │  lib/db           │  lib/scanner          │
│  (文件系统)     │  (LevelDB/SQLite) │  (文件扫描)           │
└─────────────────────────────────────────────────────────────┘

Rust 版本架构:
┌─────────────────────────────────────────────────────────────┐
│                    syncthing-api (Axum)                     │
├─────────────────────────────────────────────────────────────┤
│                   syncthing-sync (骨架)                     │
├─────────────────────────────────────────────────────────────┤
│  bep-protocol  │  syncthing-net (Iroh)                      │
│  (自定义格式)   │  (QUIC/P2P) - 完全不同的传输层               │
├─────────────────────────────────────────────────────────────┤
│  syncthing-fs  │  syncthing-db (KV抽象)  │  syncthing-core   │
│  (异步文件)     │  (内存+磁盘缓存)         │  (核心类型)        │
└─────────────────────────────────────────────────────────────┘
```

---

## 二、协议兼容性详细分析

### 2.1 Device ID 实现差异

**Go 原版** (`lib/protocol/deviceid.go`):
```go
// 32字节 SHA-256 哈希
type DeviceID [DeviceIDLength]byte  // DeviceIDLength = 32

// 字符串格式: Base32 + Luhn校验码 + 分块
// 示例: AAAAAAA-BBBBBBB-CCCCCCC-DDDDDDD-EEEEEEE-FFFFFFF-GGGGGGG-HHHHHHH
func (n DeviceID) String() string {
    id := base32.StdEncoding.EncodeToString(n[:])
    id = luhnify(id)      // 添加校验码
    id = chunkify(id)     // 分块显示
    return id
}
```

**Rust 版本** (`crates/syncthing-core/src/types.rs`):
```rust
// 同样是32字节
pub struct DeviceId([u8; 32]);

// 字符串格式: Hex编码 + 简单分块
// 示例: AAAAAAA-BBBBBBB-CCCCCCC-DDDDDDD-EEEEEEE-FFFFFFF-GGGGGGG-HHHHHHH
impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = hex::encode(&self.0[..28]).to_uppercase();
        // 7字符分块...
    }
}
```

**兼容性评估**: ⚠️ **部分兼容**
- 二进制格式相同（都是32字节）
- **字符串格式不同**: Go使用Base32+Luhn校验，Rust使用Hex
- Rust版本缺少 Luhn 校验码实现
- 设备ID字符串无法直接互通

### 2.2 BEP 协议实现差异

**Go 原版**:
- 使用 Protocol Buffer 3 标准格式
- `proto/bep/bep.proto` 定义完整消息结构
- Hello 消息使用 Magic Number: `0x2EA7D90B`
- 支持消息压缩 (LZ4)
- TLS 1.2+ 证书认证

**Rust 版本** (`crates/bep-protocol/src/messages.rs`):
```rust
// 自定义 XDR 风格编码，非 protobuf
pub const BEP_MAGIC: &[u8] = b"BEP/1.0\n";

pub struct Hello {
    pub device_name: String,
    pub client_name: String,
    pub client_version: String,
}
```

**关键差异**:

| 特性 | Go 原版 | Rust 版本 | 兼容 |
|------|---------|-----------|------|
| 序列化格式 | Protocol Buffers | 自定义 XDR | ❌ |
| Hello Magic | `0x2EA7D90B` (4字节) | `"BEP/1.0\n"` (8字节) | ❌ |
| 消息类型 | 标准 protobuf | 自定义枚举 | ❌ |
| 压缩支持 | LZ4 | 无 | ❌ |
| TLS版本 | 1.2+ | 1.2+ (rustls) | ✅ |

**兼容性评估**: ❌ **完全不兼容**
- 两者无法建立有效连接
- 需要重写 Rust 版本的协议层以使用 protobuf

### 2.3 版本向量 (Version Vector)

**Go 原版**: 标准版本向量实现

**Rust 版本** (`crates/syncthing-core/src/version_vector.rs`):
```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionVector {
    counters: Vec<(DeviceId, u64)>,
}
```

**兼容性评估**: ✅ **逻辑兼容，格式可能不同**
- 算法逻辑相同（支配/冲突检测）
- 序列化格式未验证是否与 Go 相同

---

## 三、网络层对比

### 3.1 传输协议

| 特性 | Go 原版 | Rust 版本 |
|------|---------|-----------|
| 传输层 | TCP + TLS | QUIC (Iroh) |
| NAT穿透 | UPnP, NAT-PMP, PCP | Iroh 内置 |
| 中继 | 自定义 relay 协议 | Iroh 中继 |
| 发现 | 本地广播 + 全球 DHT | Iroh DHT |
| 地址格式 | `tcp://host:port`, `relay://...` | Iroh Node ID (32字节) |

**关键发现**:
- Rust 版本使用 **Iroh** 库实现 P2P 网络
- 完全抛弃了原版的 TCP+TLS 连接方式
- 这是最大的架构差异

### 3.2 发现协议

**Go 原版** (`lib/discover/discover.go`):
```go
type Finder interface {
    Lookup(ctx context.Context, deviceID protocol.DeviceID) (address []string, err error)
    Error() error
    Cache() map[protocol.DeviceID]CacheEntry
}
```
- 支持本地广播发现 (IPv4/IPv6)
- 支持全球 DHT 发现
- 返回地址列表 (IP:Port)

**Rust 版本** (`crates/syncthing-net/src/discovery.rs`):
```rust
#[async_trait]
pub trait Discovery: Send + Sync {
    async fn lookup(&self, device: &DeviceId) -> Result<Vec<String>>;
    async fn announce(&self, device: &DeviceId, addresses: Vec<String>) -> Result<()>;
}
```
- 使用 Iroh 的发现机制
- 接口兼容但实现完全不同

---

## 四、数据存储对比

### 4.1 数据库实现

**Go 原版**:
```
internal/db/
├── sqlite/          # 新的 SQLite 实现
├── olddb/           # 旧 LevelDB 实现
└── interface.go     # 存储接口
```

**Rust 版本**:
```
crates/syncthing-db/src/
├── kv.rs           # KV 存储抽象
├── store.rs        # 块存储实现
├── metadata.rs     # 元数据存储
└── block_cache.rs  # 块缓存
```

**兼容性评估**: ❌ **不兼容**
- Go 使用 SQLite/LevelDB
- Rust 使用自定义 KV 抽象
- 数据库文件格式完全不同

---

## 五、API 兼容性

### 5.1 REST API 端点对比

**Go 原版** (部分端点):
- `/rest/system/status`
- `/rest/system/config`
- `/rest/db/status?folder=...`
- `/rest/db/completion?folder=...&device=...`

**Rust 版本** (`crates/syncthing-api/src/rest.rs`):
```rust
.route("/rest/folders", get(list_folders).post(create_folder))
.route("/rest/devices", get(list_devices).post(add_device))
.route("/rest/status", get(get_status))
.route("/rest/health", get(health_check))
```

**兼容性评估**: ⚠️ **部分兼容**
- 基本 CRUD 端点相似
- 返回数据结构可能不同
- Rust 版本端点较少

---

## 六、关键不兼容点汇总

### 6.1 阻塞性不兼容 (无法互通)

| 问题 | 严重程度 | 修复工作量 |
|------|----------|-----------|
| BEP 协议序列化格式不同 | 🔴 严重 | 高 (需重写协议层) |
| Hello Magic 不一致 | 🔴 严重 | 低 |
| 传输层不同 (TCP vs QUIC) | 🔴 严重 | 高 (需重写网络层) |
| Device ID 字符串格式 | 🟠 中 | 中 |
| 数据库格式 | 🟠 中 | 中 |

### 6.2 实现差异 (可兼容)

| 问题 | 严重程度 | 说明 |
|------|----------|------|
| API 端点差异 | 🟡 低 | 功能相似，路径可能不同 |
| 配置格式 | 🟡 低 | JSON 结构可能不同 |
| 错误消息 | 🟡 低 | 不影响功能 |

---

## 七、流程验证对比

### 7.1 连接建立流程

**Go 原版流程**:
```
1. 设备发现 -> 获取 IP:Port 列表
2. TCP 连接建立
3. TLS 握手 (双方证书认证)
4. BEP Hello 交换 (Magic: 0x2EA7D90B)
5. ClusterConfig 交换
6. Index 同步
7. 块传输 (Request/Response)
```

**Rust 版本流程**:
```
1. Iroh 发现 -> 获取 Node ID
2. QUIC 连接建立 (通过 Iroh)
3. TLS 1.3 握手 (内置)
4. BEP Hello (Magic: "BEP/1.0\n") ⚠️
5. ... (未完整实现)
```

### 7.2 文件同步流程

**Go 原版**:
```
文件夹监控 -> 扫描变更 -> 更新索引 -> 推送索引 -> 请求块 -> 传输 -> 写入
```

**Rust 版本** (当前):
```
文件夹监控 ✅ -> 扫描变更 ✅ -> 更新索引 ✅ -> (推送未实现) -> (传输未实现)
```

---

## 八、结论与建议

### 8.1 兼容性结论

| 兼容性目标 | 可行性 | 工作量 |
|------------|--------|--------|
| 与 Go Syncthing 互通 | ❌ **极低** | 需重写 60%+ 代码 |
| 配置文件兼容 | ⚠️ 可能 | 需适配格式差异 |
| API 兼容 | ⚠️ 可能 | 需调整端点和响应 |
| 独立运行 | ✅ 可行 | 当前已实现 |

### 8.2 关键问题

1. **协议不兼容**: Rust 版本使用自定义 XDR 格式，而非标准 protobuf
2. **传输层不同**: Iroh QUIC 与原版 TCP+TLS 完全不兼容
3. **设备ID格式**: 字符串表示不同，用户无法直接使用
4. **未完成集成**: 同步流程未完整实现

### 8.3 建议

#### 短期 (如需要与原版互通):
1. 重写 BEP 协议层，使用 protobuf 标准格式
2. 实现原版 TCP+TLS 传输层作为可选方案
3. 统一 Device ID 字符串格式 (Base32 + Luhn)

#### 中期:
1. 实现完整的同步流程 (Push/Pull)
2. 添加与原版 Syncthing 的集成测试

#### 长期:
1. 保持独立发展，不追求与原版互通
2. 专注于 Rust 生态和 Iroh 网络的优势

---

**总体评估**: Rust 版本是一个**架构完全不同的独立实现**，当前与 Go 原版 **不具备互通能力**。
