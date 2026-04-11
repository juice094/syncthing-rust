# Discovery Module Implementation Summary

## 概述

成功实现了 Syncthing 的全局和本地设备发现功能，完全基于 Go 原版代码 (`lib/discover/*.go`) 理解协议并实现。

## 实现文件

### 1. `crates/syncthing-net/src/local_discovery.rs`
本地发现实现，支持 IPv4 广播和 IPv6 多播。

**关键特性:**
- **多播地址:**
  - IPv4: `224.0.0.0:21027`
  - IPv6: `[ff32::5223]:21027`
- **广播间隔:** 30秒 (`BROADCAST_INTERVAL`)
- **缓存时间:** 90秒 (`CACHE_LIFE_TIME`)
- **Magic Number:** `0x2EA7D90B` (与 BEP 相同)
- **消息格式:** Protobuf `Announce` 消息，包含设备ID、地址列表和实例ID

**主要结构:**
```rust
pub struct LocalDiscovery {
    my_id: DeviceId,
    cache: Arc<RwLock<HashMap<DeviceId, CacheEntry>>>,
    socket: Arc<UdpSocket>,
    instance_id: i64,
    ...
}

pub struct Announce {
    id: Vec<u8>,          // 设备ID (32 bytes)
    addresses: Vec<String>, // 地址列表
    instance_id: i64,     // 实例ID (每次重启随机)
}
```

**核心功能:**
- `new_v4()` / `new_v6()`: 创建 IPv4/IPv6 发现客户端
- `start_announcement_task()`: 后台广播任务
- `start_receive_task()`: 后台接收处理任务
- `LocalDiscoveryService`: 统一服务，同时管理 IPv4 和 IPv6

---

### 2. `crates/syncthing-net/src/global_discovery.rs`
全局发现实现，使用 HTTPS 与发现服务器通信。

**关键特性:**
- **默认服务器:** `https://discovery.syncthing.net/v2/`
- **重注册间隔:** 30分钟 (`DEFAULT_REANNOUNCE_INTERVAL`)
- **错误重试间隔:** 5分钟 (`ANNOUNCE_ERROR_RETRY_INTERVAL`)
- **请求超时:** 30秒
- **协议:** HTTPS POST 注册，HTTPS GET 查询
- **认证:** 可选客户端证书认证

**主要结构:**
```rust
pub struct GlobalDiscoveryClient {
    server: String,
    device_id: DeviceId,
    announce_client: Client,  // 带证书
    query_client: Client,     // 无证书
    no_announce: bool,
    no_lookup: bool,
    ...
}

pub struct Announcement {
    addresses: Vec<String>,
}

pub struct GlobalDiscoveryService {
    device_id: DeviceId,
    clients: Vec<Arc<GlobalDiscoveryClient>>,  // 支持多服务器
    cache: Arc<RwLock<HashMap<DeviceId, CacheEntry>>>,
    ...
}
```

**核心功能:**
- `GlobalDiscoveryBuilder`: 构建器模式配置服务
- `lookup()`: 查询设备地址
- `announce()`: 注册设备地址
- 支持多服务器配置
- URL 选项解析（支持 `?insecure=true`, `?noannounce=true` 等）

---

### 3. `crates/syncthing-net/src/discovery.rs`
统一发现管理器，整合所有发现机制。

**关键特性:**
- 多级缓存策略
- 多源聚合（本地 + 全局 + DHT）
- 可配置的 TTL
- 向后兼容 Iroh/DHT 发现

**主要结构:**
```rust
pub struct DiscoveryManager {
    device_id: DeviceId,
    config: DiscoveryConfig,
    local_discovery: Option<Arc<LocalDiscoveryService>>,
    global_discovery: Option<Arc<GlobalDiscoveryService>>,
    iroh_discovery: Option<Arc<IrohDiscovery>>,
    cache: Arc<RwLock<HashMap<DeviceId, CacheEntry>>>,
    ...
}

pub struct DiscoveryConfig {
    pub local_discovery_v4: bool,
    pub local_discovery_v6: bool,
    pub global_discovery: bool,
    pub local_discovery_port: u16,
    pub local_discovery_v6_addr: String,
    pub global_discovery_servers: Vec<String>,
    pub cache_ttl: Duration,
    ...
}
```

**核心功能:**
- `DiscoveryManager::new()`: 异步初始化所有发现服务
- `start()`: 启动所有后台任务
- `lookup()`: 统一查询接口，按优先级（本地 -> 全局 -> DHT）
- `announce()`: 统一注册接口
- 智能缓存合并

---

### 4. `crates/syncthing-net/src/lib.rs` 更新
导出所有新的发现类型。

```rust
pub use discovery::{
    DiscoveryManager, DiscoveryConfig, 
    IrohDiscovery, MockDhtDiscovery, DhtDiscovery,
    create_discovery_manager,
};

pub use local_discovery::{
    LocalDiscovery, LocalDiscoveryService,
    DEFAULT_DISCOVERY_PORT, DEFAULT_IPV4_BROADCAST, DEFAULT_IPV6_MULTICAST,
    BROADCAST_INTERVAL, CACHE_LIFE_TIME,
    DISCOVERY_MAGIC, V13_MAGIC,
};

pub use global_discovery::{
    GlobalDiscoveryService, GlobalDiscoveryClient, GlobalDiscoveryBuilder,
    Announcement, DEFAULT_DISCOVERY_SERVER, DEFAULT_REANNOUNCE_INTERVAL,
};
```

---

## 依赖更新 (`Cargo.toml`)

新增依赖:
```toml
prost = { workspace = true }
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
rand = { workspace = true }
url = "2.5"
```

---

## 测试覆盖

所有测试通过 (16 tests):

```bash
$ cargo test -p syncthing-net discovery

running 16 tests
test local_discovery::tests::test_filter_undialable ... ok
test global_discovery::tests::test_sanitize_addresses ... ok
test discovery::tests::test_discovery_config_default ... ok
test discovery::tests::test_mock_dht_discovery ... ok
test global_discovery::tests::test_announcement_serialize ... ok
test global_discovery::tests::test_global_discovery_builder ... ok
test discovery::tests::test_discovery_manager_new ... ok
test global_discovery::tests::test_parse_options ... ok
test discovery::tests::test_discovery_manager_cache ... ok
test local_discovery::tests::test_cache_entry_validity ... ok
test discovery::tests::test_iroh_discovery ... ok
test local_discovery::tests::test_process_addresses ... ok
test discovery::tests::test_discovery_manager_announce ... ok
test local_discovery::tests::test_announce_encode_decode ... ok
test local_discovery::tests::test_local_discovery_new ... ok
test local_discovery::tests::test_local_discovery_service ... ok

test result: ok. 16 passed; 0 failed
```

---

## Discovery Trait 实现

所有发现实现都实现了 `Discovery` trait:

```rust
#[async_trait]
pub trait Discovery: Send + Sync {
    /// 查找设备地址
    async fn lookup(&self, device: &DeviceId) -> Result<Vec<String>>;
    
    /// 宣布设备地址
    async fn announce(&self, device: &DeviceId, addresses: Vec<String>) -> Result<()>;
    
    /// 启动周期性宣布
    async fn start_periodic_announce(
        &self,
        device: DeviceId,
        addresses: Vec<String>,
        interval_secs: u64,
    ) -> Result<Box<dyn AnnouncementHandle>>;
}
```

---

## 与 ConnectionManager 集成

`ConnectionManager` 通过 `Discovery` trait 使用发现服务:

```rust
pub struct ConnectionManager {
    pool: ConnectionPool,
    transport: Arc<dyn Transport>,
    discovery: Arc<dyn Discovery>,  // 使用 Discovery trait
    config: ConnectionConfig,
    ...
}

// 在连接时查找地址
async fn get_connection(&self, device: &DeviceId) -> Result<Box<dyn BepConnection>> {
    let addresses = self.discovery.lookup(device).await?;
    // 尝试连接每个地址...
}
```

---

## 功能验证

- ✅ **本地发现能广播自身地址**
  - `LocalDiscovery::start_announcement_task()` 每30秒发送广播
  - Protobuf 编码的 Announce 消息

- ✅ **能解析其他设备的发现消息**
  - `LocalDiscovery::start_receive_task()` 监听 UDP 端口
  - 解析 Announce 消息并更新缓存

- ✅ **全局发现能注册和查询**
  - `GlobalDiscoveryClient::announce()` HTTPS POST
  - `GlobalDiscoveryClient::lookup()` HTTPS GET

---

## 设计决策

1. **使用 `url` crate**: 处理复杂的 URL 解析（特别是 IPv6 地址）
2. **使用 `prost`**: Protobuf 编码/解码本地发现消息
3. **使用 `reqwest`**: 现代异步 HTTP 客户端，支持 HTTP/2
4. **多服务器支持**: GlobalDiscoveryService 支持多个发现服务器
5. **缓存策略**: 独立缓存每类发现结果，支持 TTL
6. **Builder 模式**: GlobalDiscoveryBuilder 简化配置

---

## 后续改进建议

1. 添加证书认证支持（客户端证书）
2. 实现 IPv6 link-local 地址检测
3. 添加更多单元测试覆盖错误处理路径
4. 实现发现统计和监控指标
5. 添加中继地址处理
