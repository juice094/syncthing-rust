# Syncthing Rust 功能实现开发计划

**制定日期**: 2026-04-04  
**参考架构**: Syncthing Go 原版 (main分支)  
**目标**: 实现可运行的端到端文件同步

---

## 一、参考原版架构分析

### 1.1 Go 原版关键架构

```
┌─────────────────────────────────────────────────────────────────┐
│                        lib/api (REST API)                       │
├─────────────────────────────────────────────────────────────────┤
│                        lib/model (Model)                        │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  folder.go: 文件夹管理，扫描/拉取循环                      │   │
│  │  - scanLoop(): 定期扫描                                  │   │
│  │  - pullLoop(): 拉取循环                                  │   │
│  └─────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│                      lib/protocol (BEP协议)                     │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  protocol.go: 连接管理，消息分发                          │   │
│  │  - rawConnection: 底层连接                                │   │
│  │  - Model interface: 回调接口                              │   │
│  └─────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│                     lib/connections (网络)                      │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  service.go: 连接服务                                    │   │
│  │  - TCP listener/dialer                                  │   │
│  │  - TLS handshake                                        │   │
│  │  - 连接优先级管理                                         │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### 1.2 关键接口定义 (Go原版)

**Model Interface** (协议层回调):
```go
type Model interface {
    Index(conn Connection, idx *Index) error
    IndexUpdate(conn Connection, idxUp *IndexUpdate) error  
    Request(conn Connection, req *Request) (RequestResponse, error)
    ClusterConfig(conn Connection, config *ClusterConfig) error
    Closed(conn Connection, err error)
    DownloadProgress(conn Connection, p *DownloadProgress) error
}
```

**Connection Interface**:
```go
type Connection interface {
    Index(ctx context.Context, idx *Index) error
    IndexUpdate(ctx context.Context, idxUp *IndexUpdate) error
    Request(ctx context.Context, req *Request) ([]byte, error)
    ClusterConfig(config *ClusterConfig, passwords map[string]string)
    // ...
}
```

### 1.3 BEP 协议消息流

```
连接建立:
  1. TCP 连接
  2. TLS 握手 (双方证书验证)
  3. Hello 交换 (Magic: 0x2EA7D90B)
  4. ClusterConfig 交换 (共享文件夹配置)
  5. Index 交换 (完整文件索引)

正常运行:
  - IndexUpdate (增量更新)
  - Request/Response (块请求/响应)
  - DownloadProgress (下载进度)
```

---

## 二、开发任务分解

### Phase 1: BEP 协议标准化 (Agent-Protocol)

**目标**: 实现标准 BEP 协议，与 Go 原版兼容

**任务清单**:
1. [ ] 引入 protobuf 代码生成 (prost)
2. [ ] 从 `proto/bep/bep.proto` 生成 Rust 代码
3. [ ] 重写 Hello 消息 (Magic: 0x2EA7D90B)
4. [ ] 实现消息编码/解码 (protobuf + 长度头)
5. [ ] 实现消息压缩 (LZ4)
6. [ ] 更新测试用例

**参考文件**:
- `syncthing-main/proto/bep/bep.proto`
- `syncthing-main/lib/protocol/bep_hello.go`

**验收标准**:
- 能够解析 Go 原版发送的 Hello 消息
- 能够生成 Go 原版可解析的消息
- 所有单元测试通过

---

### Phase 2: TCP+TLS 传输层 (Agent-Network)

**目标**: 替换 Iroh，实现标准 TCP+TLS 传输

**任务清单**:
1. [ ] 实现 TcpTransport (替换 IrohTransport)
2. [ ] 实现 TCP listener
3. [ ] 实现 TCP dialer
4. [ ] TLS 证书生成和管理
5. [ ] TLS 握手 (双向认证)
6. [ ] 设备ID从证书提取
7. [ ] 连接池管理

**参考文件**:
- `syncthing-main/lib/connections/service.go`
- `syncthing-main/lib/connections/tcp_dialer.go`
- `syncthing-main/lib/connections/tcp_listener.go`

**关键实现**:
```rust
pub struct TcpTransport {
    listeners: Vec<TcpListener>,
    tls_config: Arc<rustls::ServerConfig>,
    device_id: DeviceId,
}

impl Transport for TcpTransport {
    async fn connect(&self, addr: &str) -> Result<Box<dyn BepConnection>>;
    async fn listen(&self, bind_addr: &str) -> Result<Box<dyn ConnectionListener>>;
}
```

**验收标准**:
- 能够与 Go Syncthing 建立 TCP 连接
- TLS 握手成功
- 设备ID正确提取

---

### Phase 3: 同步模型 (Agent-Sync)

**目标**: 实现 Model 接口，完成同步状态机

**任务清单**:
1. [ ] 定义 Model trait (对应 Go 的 Model interface)
2. [ ] 实现 Index 处理 (接收远程索引)
3. [ ] 实现 IndexUpdate 处理 (接收增量更新)
4. [ ] 实现 Request 处理 (响应块请求)
5. [ ] 实现 pull 循环 (检测变更并拉取)
6. [ ] 实现 push 循环 (发送索引更新)
7. [ ] 文件夹状态管理 (Scanning/Syncing/Idle)

**参考文件**:
- `syncthing-main/lib/model/folder.go`
- `syncthing-main/lib/model/folder_sendrecv.go`

**关键 trait**:
```rust
#[async_trait]
pub trait Model: Send + Sync {
    async fn index(&self, conn: &dyn Connection, idx: &Index) -> Result<()>;
    async fn index_update(&self, conn: &dyn Connection, idx_up: &IndexUpdate) -> Result<()>;
    async fn request(&self, conn: &dyn Connection, req: &Request) -> Result<Vec<u8>>;
    async fn cluster_config(&self, conn: &dyn Connection, cfg: &ClusterConfig) -> Result<()>;
    async fn closed(&self, conn: &dyn Connection, err: Option<&Error>);
}
```

**验收标准**:
- 能够接收并处理远程索引
- 能够检测需要同步的文件
- 能够请求并接收文件块
- 文件夹状态正确转换

---

### Phase 4: 集成与 CLI (Agent-Integration)

**目标**: 集成所有模块，完善 CLI

**任务清单**:
1. [ ] 集成 Protocol + Network + Sync
2. [ ] 更新 syncthing run 命令启动所有服务
3. [ ] 实现设备连接管理
4. [ ] 实现文件夹同步调度
5. [ ] 添加日志和监控
6. [ ] 端到端测试 (两台设备)

**验收标准**:
- `syncthing run` 启动完整服务
- 两台 Rust 实例可以同步文件
- 能够与 Go Syncthing 互通

---

## 三、开发顺序与依赖

```
Phase 1: BEP 协议 (Agent-Protocol)
    │
    ▼
Phase 2: TCP+TLS 传输 (Agent-Network)
    │
    ▼
Phase 3: 同步模型 (Agent-Sync)
    │
    ▼
Phase 4: 集成测试 (Agent-Integration)
```

**并行开发机会**:
- Phase 1 和 Phase 2 可以并行
- Phase 3 依赖 Phase 1 和 2
- Phase 4 依赖所有前置阶段

---

## 四、关键设计决策

### 4.1 协议兼容性

**决策**: 完全兼容 Go 原版 BEP 协议

**理由**:
- 可以与现有 Syncthing 网络互通
- 利用成熟的协议设计
- 用户无缝迁移

### 4.2 传输层选择

**决策**: TCP+TLS (标准实现)

**理由**:
- Go 原版使用 TCP+TLS
- 更好的兼容性
- 证书管理成熟

### 4.3 异步运行时

**决策**: 继续使用 Tokio

**理由**:
- Rust 标准异步运行时
- 生态丰富
- 已有代码基于 Tokio

---

## 五、风险评估

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|----------|
| protobuf 兼容性问题 | 中 | 高 | 与原版进行二进制对比测试 |
| TLS 证书格式差异 | 低 | 高 | 使用标准 x509 解析 |
| 性能差异 | 中 | 中 | 早期进行基准测试 |
| 并发 Bug | 中 | 高 | 大量测试，使用 loom 检测 |

---

## 六、验收检查清单

### Phase 1 验收
- [ ] 生成的 protobuf 代码编译通过
- [ ] Hello 消息编解码测试通过
- [ ] Index/Request/Response 消息测试通过
- [ ] 与 Go 原版消息格式一致

### Phase 2 验收
- [ ] TCP 监听和连接建立
- [ ] TLS 握手成功
- [ ] 设备ID正确提取
- [ ] 连接池管理正常

### Phase 3 验收
- [ ] Model trait 所有方法实现
- [ ] 索引处理和差异计算
- [ ] 块请求和响应
- [ ] 文件夹状态管理

### Phase 4 验收
- [ ] 两台 Rust 实例文件同步
- [ ] 与 Go Syncthing 互通
- [ ] 端到端测试通过
- [ ] 性能基准达标

---

**下一步**: 启动 Phase 1 (Agent-Protocol) 和 Phase 2 (Agent-Network) 并行开发
