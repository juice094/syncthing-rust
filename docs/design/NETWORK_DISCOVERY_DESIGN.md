# Syncthing-Rust 自建网络发现层设计文档

> **版本**: v0.1.0-draft  
> **日期**: 2026-04-20  
> **状态**: 设计评审中  
> **参考源码**: syncthing-go (`lib/discover`, `lib/stun`, `lib/upnp`, `lib/relay`, `lib/beacon`), Tailscale (`derp/`, `disco/`, `net/netcheck/`, `portmapper/`)

---

## 1. 设计目标与背景

### 1.1 问题陈述

当前 Rust 实现通过 **Tailscale 虚拟网 (`100.x.x.x`)** 进行设备间通信。这带来两个致命问题：

1. **外部依赖过重**: 用户必须安装并配置 Tailscale，且云端控制平面故障（如当前格雷节点掉线 9h+）直接导致所有连接中断。
2. **非开箱即用**: 产品定位为"装完即用"的 P2P 同步工具，捆绑第三方 VPN 与这一理念相悖。

### 1.2 设计目标

实现**不依赖任何外部 VPN/控制平面**的网络发现与 NAT 穿透层，达到以下能力：

| 能力 | 目标 | 优先级 |
|------|------|--------|
| 局域网自动发现 | 同网段设备秒级发现，零配置 | P0 |
| 公网直连 (NAT穿透) | 支持 Cone NAT 的 P2P 直连 | P0 |
| 端口映射 (UPnP/NAT-PMP) | 自动申请路由器端口映射 | P1 |
| 中继回退 (Relay) | 穿透失败时通过公共中继传输 | P1 |
| 全局发现服务器 | 可选接入 Syncthing 官方发现服务 | P2 |
| DERP 自建中继 | 自建轻量级 TCP 中继集群 | P3 |

### 1.3 核心设计原则

- **兼容性优先**: 优先兼容 Syncthing 官方协议（Local/Global Discovery、Relay Protocol），确保可与 Go 版节点互通。
- **渐进式实现**: 先做 Local Discovery + UPNP → 再做 STUN + Relay → 最后 Global Discovery + DERP。
- **零配置默认**: 所有功能默认开启，用户无需手动配置即可在大多数网络环境下工作。
- **模块化隔离**: 每个子系统独立 crate/module，通过事件总线通信，不耦合现有 BEP 连接层。

---

## 2. 架构总览

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         syncthing-net crate                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐ │
│  │   Local     │  │   Global    │  │   Address   │  │   Connection    │ │
│  │  Discovery  │  │  Discovery  │  │   Manager   │  │    Manager      │ │
│  │  (UDP 21027)│  │ (HTTPS/mTLS)│  │             │  │  (TLS + BEP)    │ │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └─────────────────┘ │
│         │                │                │                              │
│  ┌──────▼──────┐  ┌──────▼──────┐  ┌──────▼──────┐                      │
│  │    STUN     │  │    UPNP     │  │    Relay    │                      │
│  │  (UDP/Port) │  │(SSDP+SOAP)  │  │ (TCP Relay) │                      │
│  └─────────────┘  └─────────────┘  └─────────────┘                      │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                     Event Bus (tokio::broadcast)                 │    │
│  │  DeviceDiscovered / AddressesUpdated / NatTypeDetected / ...     │    │
│  └─────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      syncthing-core / syncthing-sync                     │
│                    (消费事件，触发 BEP 连接尝试)                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### 2.1 数据流

1. **发现阶段**: Local/Global Discovery 收集潜在地址 → 写入 Address Manager
2. **检测阶段**: STUN 检测 NAT 类型 + UPNP 申请端口映射 → 更新可访问地址列表
3. **连接阶段**: Connection Manager 按优先级尝试地址（直连 → 中继），成功后走 BEP/TLS
4. **维护阶段**: 定期保活、重注册、地址刷新

---

## 3. Local Discovery（本地发现）

### 3.1 协议规范（与 syncthing-go 兼容）

| 参数 | 值 |
|------|-----|
| **Magic** | `0x2EA7D90B`（与 BEP Hello Magic 相同） |
| **端口** | `21027` |
| **IPv4 目标** | 各网卡计算出的子网广播地址，fallback 到 `255.255.255.255` |
| **IPv6 目标** | `[ff12::8384:21027]:21027` |
| **传输层** | UDP |
| **消息格式** | protobuf `Announce` |
| **广播间隔** | `30s` |
| **缓存有效期** | `90s`（3 × 广播间隔） |
| **触发条件** | 定时广播 + 发现新设备时立即广播 |

### 3.2 Protobuf 消息定义

```protobuf
message Announce {
    bytes  id          = 1;   // Device ID (SHA-256 原始 32 bytes)
    repeated string addresses = 2;  // 如 "tcp://192.168.1.10:22000"
    int64  instance_id = 3;   // 进程实例标识，重启后变化
}
```

### 3.3 网卡枚举策略

参考 `lib/beacon/broadcast.go`：

1. 遍历所有网卡，筛选条件：
   - `FlagRunning` 必须置位
   - `FlagBroadcast`（IPv4）或 `FlagMulticast`（IPv6）必须置位
   - Android 平台跳过 `FlagPointToPoint`（蜂窝网络）
2. 获取网卡地址，筛选 `IsGlobalUnicast() && To4() != nil`
3. 计算广播地址：`ip | ^mask`
4. 若无可用网卡，fallback 到全局广播 `255.255.255.255`

### 3.4 Rust 实现要点

```rust
// 核心结构
pub struct LocalDiscovery {
    port: u16,                    // 21027
    device_id: DeviceId,
    instance_id: u64,             // random
    announce_addrs: Vec<String>,  // 当前可announce的地址
    cache: Arc<Mutex<HashMap<DeviceId, CachedEntry>>>,
    broadcast_interval: Duration, // 30s
}

// 双工运行
impl LocalDiscovery {
    pub async fn run(&self) {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(16);
        tokio::select! {
            _ = self.broadcast_loop(tx) => {},
            _ = self.listen_loop(rx) => {},
        }
    }
}
```

**关键注意点**：
- Windows 上 UDP socket 需要 `SO_BROADCAST` 权限（`tokio::net::UdpSocket::set_broadcast(true)`）
- IPv6 multicast 需要 `IPV6_JOIN_GROUP` + `IPV6_MULTICAST_HOPS=1`
- 使用 `tokio::net::UdpSocket`，bufsize `65536`

---

## 4. Global Discovery（全局发现）

### 4.1 协议规范

| 参数 | 值 |
|------|-----|
| **协议** | HTTPS |
| **默认服务器** | `https://discovery.syncthing.net/v2/` |
| **认证方式** | mTLS（客户端证书 = 设备证书） |
| **Announce 间隔** | `30min` |
| **请求超时** | `30s` |
| **Announce 方法** | POST，Body 为 JSON `{ "addresses": [...] }` |
| **Query 方法** | GET `?device=<device_id>`，返回 JSON 地址列表 |
| **错误重试** | 失败时 `5min` 后重试 |

### 4.2 mTLS 认证

Global Discovery 使用与 BEP 连接**相同的 TLS 证书**进行双向认证：

- 客户端发起 HTTPS 连接时提供设备证书（`cert.pem`）
- 验证服务端证书链
- 地址中设备 ID 由证书中公钥的 SHA-256 派生，确保身份绑定

### 4.3 Rust 实现要点

```rust
pub struct GlobalDiscovery {
    server_url: String,
    device_id: DeviceId,
    tls_config: Arc<rustls::ClientConfig>,
    announce_interval: Duration, // 30min
    retry_interval: Duration,    // 5min
}

impl GlobalDiscovery {
    async fn announce(&self, addresses: &[String]) -> Result<()> {
        let client = reqwest::ClientBuilder::new()
            .use_preconfigured_tls(self.tls_config.clone())
            .timeout(Duration::from_secs(30))
            .build()?;
        
        let body = json!({ "addresses": addresses });
        client.post(&format!("{}?device={}", self.server_url, self.device_id))
            .json(&body)
            .send().await?;
        Ok(())
    }
    
    async fn query(&self, target: DeviceId) -> Result<Vec<String>> {
        // GET ?device=<id>
    }
}
```

---

## 5. STUN / NAT 类型检测

### 5.1 协议规范

| 参数 | 值 |
|------|-----|
| **协议** | RFC 5389 STUN |
| **默认服务器** | Syncthing 内置: `stun.syncthing.net:3478` 等 |
| **检测目标** | 确定 NAT 类型，获取公网映射地址 |
| **重试间隔** | `5min` |
| **保活** | 自适应退避，维持端口映射不超时 |

### 5.2 NAT 类型分类与穿透可行性

参考 `go-stun` 库：

| NAT 类型 | 穿透可行性 | 策略 |
|----------|-----------|------|
| `NATNone` | ✅ 公网 IP | 直接暴露 |
| `NATFullCone` | ✅ 容易 | 任何外部地址可直接连接 |
| `NATRestricted` | ✅ 可以 | 需先发送包到对端 |
| `NATPortRestricted` | ✅ 可以 | 需先发送包到对端端口 |
| `NATSymmetric` | ❌ 困难 | 需依赖 UPNP 或 Relay |
| `NATSymmetricUDPFirewall` | ❌ 困难 | 需依赖 UPNP 或 Relay |
| `NATBlocked` | ❌ 不可能 | 只能 Relay |
| `NATUnknown` | ⚠️ 未知 | 尝试穿透，失败 fallback |

### 5.3 Rust 实现方案

**推荐库**: `stun` crate（tokio 异步支持）或自研轻量实现。

```rust
pub struct StunClient {
    servers: Vec<SocketAddr>,
    local_socket: UdpSocket,
}

impl StunClient {
    /// 检测 NAT 类型 + 获取公网映射地址
    pub async fn discover(&self) -> Result<NatInfo> {
        // 1. 向 STUN 服务器 A 发送 Binding Request
        // 2. 向 STUN 服务器 B 发送 Binding Request
        // 3. 对比返回的 mapped address
        // 4. 判断 NAT 类型
    }
}

pub struct NatInfo {
    pub nat_type: NatType,
    pub mapped_addr: Option<SocketAddr>,  // 公网映射地址
    pub local_addr: SocketAddr,
}
```

---

## 6. UPnP / NAT-PMP 端口映射

### 6.1 UPnP 协议流程

**SSDP 发现**（UDP 多播）：
```
M-SEARCH * HTTP/1.1
HOST: 239.255.255.250:1900
MAN: "ssdp:discover"
MX: 3
ST: urn:schemas-upnp-org:device:InternetGatewayDevice:1
```

**设备描述获取**: HTTP GET 返回 XML，解析 `controlURL`

**SOAP 端口映射**:
```xml
<AddPortMapping>
  <NewRemoteHost></NewRemoteHost>
  <NewExternalPort>22000</NewExternalPort>
  <NewProtocol>TCP</NewProtocol>
  <NewInternalPort>22000</NewInternalPort>
  <NewInternalClient>192.168.1.10</NewInternalClient>
  <NewEnabled>1</NewEnabled>
  <NewPortMappingDescription>syncthing-rust</NewPortMappingDescription>
  <NewLeaseDuration>0</NewLeaseDuration>
</AddPortMapping>
```

### 6.2 NAT-PMP (PMP/PCP)

更简单的 Apple 协议，端口 `5351`，无需 XML/SOAP，二进制协议。

### 6.3 Rust 实现方案

**推荐库**: `igd-next` crate（支持 aync，IGDv1/IGDv2 + NAT-PMP/PCP）

```rust
use igd_next::aio::tokio::search_gateway;

pub async fn add_port_mapping(
    local_addr: SocketAddrV4,
    external_port: u16,
    lease_duration: u32,
) -> Result<()> {
    let gateway = search_gateway(Default::default()).await?;
    gateway.add_port(
        PortMappingProtocol::TCP,
        external_port,
        local_addr,
        lease_duration,
        "syncthing-rust",
    ).await?;
    Ok(())
}
```

**维护策略**:
- 初始启动时尝试映射
- 租约过半时自动续期
- 网络变化（网卡 up/down）时重新发现

---

## 7. Relay 中继协议

### 7.1 协议规范（syncthing-go 兼容）

| 参数 | 值 |
|------|-----|
| **Magic** | `0x9E79BC40` |
| **协议名** | `bep-relay` |
| **传输层** | TCP |
| **序列化** | XDR |
| **消息类型** | Ping/Pong, JoinRelayRequest, JoinSessionRequest, Response, ConnectRequest, SessionInvitation, RelayFull |

### 7.2 消息定义（XDR）

```rust
// Header: 8 bytes
//   magic: u32 (0x9E79BC40)
//   message_type: u8
//   message_length: i32

#[derive(Debug)]
pub struct Ping;
#[derive(Debug)]
pub struct Pong;

#[derive(Debug)]
pub struct JoinRelayRequest {
    pub token: Vec<u8>,  // 用于后续 JoinSession 验证
}

#[derive(Debug)]
pub struct JoinSessionRequest {
    pub from: DeviceId,
    pub to: DeviceId,
    pub token: Vec<u8>,
    pub address: String,
}

#[derive(Debug)]
pub struct SessionInvitation {
    pub from: DeviceId,
    pub key: Vec<u8>,
    pub address: String,
    pub server_socket: String,  // relay server address
    pub token: Vec<u8>,
}

#[derive(Debug)]
pub struct ConnectRequest {
    pub token: Vec<u8>,
}
```

### 7.3 中继连接流程

```
Alice (behind NAT)          Relay Server           Bob (behind NAT)
    |                           |                        |
    |-- TCP connect ----------->|                        |
    |-- JoinRelayRequest ----->|                        |
    |<-- Response(success) ----|                        |
    |                           |<-- TCP connect -------|
    |                           |<-- JoinRelayRequest --|
    |                           |--- Response(success)->|
    |                           |                        |
    |-- JoinSessionRequest ----|                        |
    |  (to=Bob, token)          |--- SessionInvitation->|
    |                           |  (from=Alice, token)   |
    |                           |                        |
    |                           |<-- JoinSessionRequest-|
    |                           |  (to=Alice, token)     |
    |<-- SessionInvitation ----|                        |
    |  (from=Bob, token)        |                        |
    |                           |                        |
    |-- ConnectRequest(token)->|                        |
    |<-- full-duplex relay --->|<-- full-duplex relay --|
```

### 7.4 Rust 实现要点

```rust
pub struct RelayClient {
    server_addr: SocketAddr,
    device_id: DeviceId,
}

impl RelayClient {
    /// 连接到中继服务器，注册为可用节点
    pub async fn join_relay(&self) -> Result<RelayConn> {
        let stream = TcpStream::connect(self.server_addr).await?;
        let mut conn = RelayConn::new(stream);
        conn.send_message(JoinRelayRequest { token: vec![] }).await?;
        let resp = conn.read_message().await?;
        // expect ResponseSuccess
        Ok(conn)
    }
    
    /// 请求与目标设备建立中继会话
    pub async fn join_session(&mut self, target: DeviceId) -> Result<SessionInvitation> {
        self.send_message(JoinSessionRequest {
            from: self.device_id,
            to: target,
            token: generate_token(),
            address: String::new(),
        }).await?;
        match self.read_message().await? {
            Message::SessionInvitation(inv) => Ok(inv),
            Message::Response(resp) if resp.code == 1 => Err(RelayError::PeerNotFound),
            _ => Err(RelayError::Unexpected),
        }
    }
}
```

---

## 8. 地址管理与连接策略

### 8.1 地址类型

```rust
#[derive(Clone, Debug, PartialEq)]
pub enum AddressType {
    /// 局域网直连地址 (e.g., tcp://192.168.1.10:22000)
    LanDirect(SocketAddr),
    /// 公网直连地址 (e.g., tcp://203.0.113.5:22000)
    WanDirect(SocketAddr),
    /// 中继地址 (e.g., relay://relay.syncthing.net:22067?id=...)
    Relay(SocketAddr, DeviceId),
    /// DERP 中继地址
    Derp(String),
}
```

### 8.2 地址收集来源

| 来源 | 地址类型 | 优先级 |
|------|---------|--------|
| Local Discovery | `LanDirect` | 最高 (延迟最低) |
| UPNP/NAT-PMP 映射 | `WanDirect` (公网) | 高 |
| STUN 映射 | `WanDirect` (公网映射) | 高 |
| Global Discovery | `WanDirect` / `LanDirect` | 中 |
| 用户配置 | 任意 | 按配置顺序 |
| Relay Pool | `Relay` | 最低 (兜底) |

### 8.3 连接尝试策略

```rust
pub async fn connect_to_device(&self, device_id: DeviceId) -> Result<BepConnection> {
    let addresses = self.address_manager.get_addresses(device_id).await;
    
    // 按优先级排序：LAN > WAN > Relay
    let sorted = self.prioritize_addresses(addresses);
    
    for addr in sorted {
        match self.try_connect(addr).await {
            Ok(conn) => {
                self.address_manager.mark_working(device_id, &addr);
                return Ok(conn);
            }
            Err(e) => {
                self.address_manager.mark_failed(device_id, &addr);
                debug!("Failed to connect via {:?}: {}", addr, e);
            }
        }
    }
    
    Err(ConnectionError::AllAddressesFailed)
}
```

### 8.4 连接后验证

成功建立 TCP 连接后：
1. 执行 TLS 握手（双方交换设备证书）
2. 执行 BEP Hello 交换（确认设备 ID 匹配）
3. 发送/接收 ClusterConfig
4. 标记该地址为可用，后续优先使用

---

## 9. 事件驱动架构

### 9.1 事件类型

```rust
#[derive(Clone, Debug)]
pub enum DiscoveryEvent {
    /// 发现新设备
    DeviceDiscovered {
        device_id: DeviceId,
        addresses: Vec<String>,
        source: DiscoverySource,
    },
    /// 设备地址更新
    AddressesUpdated {
        device_id: DeviceId,
        added: Vec<String>,
        removed: Vec<String>,
    },
    /// NAT 类型检测结果
    NatTypeDetected {
        nat_type: NatType,
        mapped_addr: Option<SocketAddr>,
    },
    /// UPNP 端口映射成功/失败
    PortMappingChanged {
        external_addr: Option<SocketAddr>,
        success: bool,
    },
    /// 中继会话就绪
    RelaySessionReady {
        device_id: DeviceId,
        invitation: SessionInvitation,
    },
}

#[derive(Clone, Debug)]
pub enum DiscoverySource {
    LocalBroadcast,
    LocalMulticast,
    GlobalDiscovery,
    UserConfig,
    RelayPool,
}
```

### 9.2 事件总线集成

```rust
pub struct DiscoveryBus {
    tx: broadcast::Sender<DiscoveryEvent>,
}

impl DiscoveryBus {
    pub fn subscribe(&self) -> broadcast::Receiver<DiscoveryEvent> {
        self.tx.subscribe()
    }
    
    pub fn publish(&self, event: DiscoveryEvent) {
        let _ = self.tx.send(event);
    }
}
```

syncthing-core 中的 Connection Manager 订阅事件，收到 `DeviceDiscovered` 或 `AddressesUpdated` 后触发连接尝试。

---

## 10. 与现有代码的集成点

### 10.1 当前架构回顾

```
crates/syncthing-net/
├── src/
│   ├── lib.rs           # 导出
│   ├── tls.rs           # TLS 配置 (已有)
│   ├── handshaker.rs    # BEP Hello (已有)
│   ├── connection.rs    # BEP Connection (已有)
│   ├── transport.rs     # TCP 监听/连接 (已有)
│   └── ...
```

### 10.2 新增模块规划

```
crates/syncthing-net/
├── src/
│   ├── lib.rs
│   ├── tls.rs
│   ├── handshaker.rs
│   ├── connection.rs
│   ├── transport.rs
│   ├── discovery/
│   │   ├── mod.rs           # 导出 + 事件总线
│   │   ├── local.rs         # UDP 广播/多播发现
│   │   ├── global.rs        # HTTPS 全局发现
│   │   ├── address_manager.rs # 地址存储与排序
│   │   └── events.rs        # DiscoveryEvent 定义
│   ├── nat/
│   │   ├── mod.rs
│   │   ├── stun.rs          # STUN 客户端
│   │   └── upnp.rs          # UPnP/NAT-PMP 映射
│   └── relay/
│       ├── mod.rs           # 导出
│       ├── client.rs        # Relay TCP 客户端
│       ├── protocol.rs      # XDR 消息定义 + 编解码
│       └── types.rs         # Relay 数据结构
```

### 10.3 依赖调整

**Cargo.toml 新增**:
```toml
[dependencies]
# Local Discovery / NAT
stun = "0.6"           # STUN 客户端
igd-next = { version = "0.15", features = ["aio_tokio"] }  # UPnP

# Global Discovery
reqwest = { workspace = true, features = ["rustls-tls"] }

# Relay Protocol
xdr = "0.3"            # XDR 序列化（或自研）

# 共享依赖
tokio = { workspace = true, features = ["net", "time", "sync"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tracing = { workspace = true }
```

---

## 11. 安全考量

### 11.1 Local Discovery 安全

- UDP 广播/多播报文**不加密**（同网段信任模型）
- 但地址信息本身不敏感，真正认证在 TLS 层
- 需要防止 announce  flooding：限制同一设备的 announce 频率

### 11.2 Global Discovery 安全

- mTLS 双向认证，地址信息与设备证书绑定
- 服务器无法伪造设备身份
- 用户可选择禁用 Global Discovery（仅局域网模式）

### 11.3 Relay 安全

- 中继服务器**只能看到加密流量**（TLS 在 Relay 隧道之上）
- 设备 ID 验证在 BEP Hello 层完成
- 防止未授权 JoinSession：token 随机生成，单次有效

---

## 12. 实现计划与里程碑

### Phase 1: 局域网发现（1-2 周）

| 任务 | 工作量 | 依赖 |
|------|--------|------|
| `discoproto` protobuf 定义（Announce） | 0.5d | - |
| UDP 广播发送/接收（IPv4） | 1d | - |
| UDP 多播发送/接收（IPv6） | 1d | - |
| 网卡枚举 + 地址计算 | 1d | - |
| 与现有 BEP 连接层集成 | 1d | - |
| 端到端测试（Rust ↔ Go 局域网） | 1d | - |

**验收标准**: 同一局域网内 Rust 节点与 Go 节点可自动发现并成功同步。

### Phase 2: NAT 检测 + 端口映射（1-2 周）

| 任务 | 工作量 | 依赖 |
|------|--------|------|
| STUN 客户端（NAT 类型检测 + 公网映射地址） | 2d | - |
| UPnP 端口映射（`igd-next` 集成） | 1.5d | - |
| NAT-PMP/PCP 端口映射 | 1d | - |
| 地址管理器（排序、去重、失败标记） | 1.5d | Phase 1 |
| 连接策略实现（LAN > WAN > Relay） | 1d | - |

**验收标准**: 在典型家用路由器（Cone NAT + UPnP）环境下，两个节点可通过公网直连。

### Phase 3: Relay 中继（2 周）

| 任务 | 工作量 | 依赖 |
|------|--------|------|
| Relay Protocol XDR 消息编解码 | 1.5d | - |
| Relay 客户端（JoinRelay + JoinSession） | 2d | - |
| 公共 Relay 池集成（syncthing 官方 relays） | 1.5d | - |
| Relay 连接作为 BEP Transport 接入 | 1d | - |
| 中继回退逻辑（直连失败后自动 Relay） | 1d | - |

**验收标准**: 在对称 NAT / 无 UPnP 环境下，可通过公共 Relay 成功同步。

### Phase 4: Global Discovery（1 周）

| 任务 | 工作量 | 依赖 |
|------|--------|------|
| HTTPS mTLS 客户端 | 1d | - |
| Announce / Query API | 1d | - |
| 与 syncthing 官方发现服务器互通测试 | 1d | - |

**验收标准**: 两个位于不同网络且无 Tailscale 的节点可通过 Global Discovery 找到对方并连接。

### Phase 5: DERP 自建中继（可选，2-3 周）

| 任务 | 工作量 | 依赖 |
|------|--------|------|
| DERP 协议 Rust 实现（参考 Tailscale） | 3-5d | - |
| 轻量级 DERP 服务端 | 2-3d | - |
| 区域节点部署与负载均衡 | 2-3d | - |

---

## 13. 配置设计

```toml
[network]
# 监听地址
listen_addresses = ["0.0.0.0:22000", "[::]:22000"]

[discovery]
# 本地发现
local_discovery_enabled = true
local_discovery_port = 21027

# 全局发现
global_discovery_enabled = true
global_discovery_servers = ["https://discovery.syncthing.net/v2/"]
global_discovery_announce_interval = "30m"

[nat]
# STUN
stun_enabled = true
stun_servers = ["stun.syncthing.net:3478", "stun.l.google.com:19302"]
stun_keepalive_interval = "5m"

# UPnP/NAT-PMP
upnp_enabled = true
natpmp_enabled = true
port_mapping_lease_duration = 3600  # 0 = 永久

[relay]
# 中继
relay_enabled = true
relay_servers = ["relay://relay.syncthing.net:22067"]
relay_without_global_discovery = false  # 无全局发现时是否使用中继
```

---

## 14. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| Windows 防火墙拦截 UDP 21027 | Local Discovery 失效 | 安装时提示用户放行，或尝试不同端口 |
| 路由器无 UPnP + Symmetric NAT | 公网直连失败 | 自动 fallback 到 Relay |
| 公共 Relay 服务器不可用 | 对称 NAT 节点无法通信 | 自建 DERP 中继集群，多 Relay 轮询 |
| SSDP XML 解析兼容性 | UPnP 发现失败 | 支持 IGDv1/IGDv2，fallback 到 NAT-PMP |
| XDR 序列化库选择 | Relay 协议兼容 | 严格对照 Go 版 XDR 布局自测 |
| mTLS 证书格式差异 | Global Discovery 认证失败 | 复用现有 BEP TLS 证书逻辑 |

---

## 15. 附录

### A. 参考资源

- **Syncthing Local Discovery**: `lib/discover/local.go`, `lib/beacon/broadcast.go`, `lib/beacon/multicast.go`
- **Syncthing Global Discovery**: `lib/discover/global.go`
- **Syncthing STUN**: `lib/stun/stun.go`
- **Syncthing UPnP**: `lib/upnp/upnp.go`
- **Syncthing Relay**: `lib/relay/protocol/protocol.go`, `lib/relay/client/
- **Tailscale DERP**: `derp/derp.go`
- **Tailscale Disco**: `disco/disco.go`
- **Tailscale Portmapper**: `portmapper/portmapper.go`

### B. 关键常数速查

```rust
pub const LOCAL_DISCOVERY_PORT: u16 = 21027;
pub const LOCAL_DISCOVERY_MAGIC: u32 = 0x2EA7D90B;
pub const LOCAL_DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);
pub const LOCAL_DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(90);

pub const GLOBAL_DISCOVERY_DEFAULT_SERVER: &str = "https://discovery.syncthing.net/v2/";
pub const GLOBAL_DISCOVERY_REANNOUNCE: Duration = Duration::from_secs(1800); // 30min
pub const GLOBAL_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

pub const STUN_RETRY_INTERVAL: Duration = Duration::from_secs(300); // 5min

pub const RELAY_MAGIC: u32 = 0x9E79BC40;
pub const RELAY_PROTOCOL_NAME: &str = "bep-relay";
```

### C. 与 Go 版互通检查清单

- [ ] Local Discovery: Go 端能收到 Rust 的 Announce，反之亦然
- [ ] Global Discovery: Rust 能用 Go 生成的证书成功 Announce/Query
- [ ] STUN: Rust 的 NAT 检测结果与 Go 端一致
- [ ] UPnP: Rust 申请的端口映射可被 Go 端使用
- [ ] Relay: Rust 客户端可连接 Go 中继服务器并成功 JoinSession
- [ ] End-to-end: 无 Tailscale 环境下，Rust ↔ Go 完整文件同步通过
