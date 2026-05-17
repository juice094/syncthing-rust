# Wave 1 任务分配

**启动时间**: 2026-04-03  
**验收截止**: 2026-04-05  
**状态**: IN PROGRESS

---

## Agent-Net-1: Iroh 连接实现

**任务ID**: NET-001  
**优先级**: P0  
**工作量**: 8 小时  
**依赖**: None

### 目标
实现基于 Iroh 的 P2P 连接建立

### 交付物
1. `crates/syncthing-net/src/transport.rs` - 完整实现
2. `tests/transport_integration.rs` - 集成测试

### 具体要求
- 实现 `Transport::connect()` - 实际连接逻辑
- 实现 `Transport::listen()` - 监听并返回 ConnectionListener
- 处理 Iroh Endpoint 创建和配置
- 支持 Device ID 验证

### 验收标准
- [ ] `cargo test -p syncthing-net` 通过
- [ ] 可以创建 Iroh endpoint
- [ ] 监听接口正常工作
- [ ] 集成测试: 两个设备可以握手

### 约束
- 使用 iroh 0.32
- 不使用 unsafe
- 所有公共函数必须有测试

---

## Agent-Net-2: BEP 消息流集成

**任务ID**: NET-002  
**优先级**: P0  
**工作量**: 8 小时  
**依赖**: NET-001

### 目标
将 BEP 协议消息与 Iroh 流集成

### 交付物
1. `crates/syncthing-net/src/connection.rs` - 完整实现
2. `tests/connection_bep.rs` - BEP 消息测试

### 具体要求
- 使用 Iroh 的 `Connection::open_bi()` 创建双向流
- 实现消息序列化/反序列化
- 支持 Index/Request/Response/DownloadProgress 消息
- 消息分片和重组

### 验收标准
- [ ] 可以通过 Iroh 流发送 BEP Index 消息
- [ ] 可以接收并解析 BEP Response 消息
- [ ] 消息顺序保证
- [ ] 集成测试: 端到端消息交换

---

## Agent-Net-3: 设备发现实现

**任务ID**: NET-003  
**优先级**: P0  
**工作量**: 6 小时  
**依赖**: None

### 目标
实现设备发现机制

### 交付物
1. `crates/syncthing-net/src/discovery.rs` - 完整实现
2. `tests/discovery_integration.rs` - 发现测试

### 具体要求
- 集成 Iroh DHT 发现
- 支持本地多播发现
- 设备地址缓存
- 定期公告刷新

### 验收标准
- [ ] 可以通过 Iroh DHT 发现设备
- [ ] 本地多播发现工作
- [ ] 地址缓存有效
- [ ] 集成测试: 设备发现成功

---

## Agent-Net-4: 连接管理器

**任务ID**: NET-004  
**优先级**: P1  
**工作量**: 6 小时  
**依赖**: NET-001

### 目标
实现连接池和生命周期管理

### 交付物
1. `crates/syncthing-net/src/manager.rs` - 连接管理器
2. `tests/manager_test.rs` - 管理器测试

### 具体要求
- 连接池（复用连接）
- 连接保活 (keepalive)
- 自动重连
- 并发连接限制

### 验收标准
- [ ] 同一设备多次连接复用
- [ ] 连接断开自动重连
- [ ] 并发连接数限制有效
- [ ] 集成测试: 100 次连接/断开循环

---

## 子代理报告模板

每个子代理交付时必须填写:

```markdown
## Agent Report: <Agent-ID>

### 任务: <Task-ID>
### 状态: DELIVERED / FAILED

### 交付文件
| 文件 | 行数 | 测试数 | 状态 |
|------|------|--------|------|
| ... | ... | ... | UNVERIFIED |

### 自测结果
- 单元测试: X/Y passed
- 文档测试: X/Y passed

### 已知问题
- [ ] 问题1
- [ ] 问题2

### 使用说明
如何编译和测试...
```

---

**注意**: 子代理交付标记为 UNVERIFIED，必须经过 Master Agent 验收测试才能合并。
