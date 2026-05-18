# 子代理集群任务分配计划

> 基于 Go 原版 (syncthing-main) 与 Rust 实现的模块对比

## 一、模块映射对比

| Go 原版模块 | Rust 对应模块 | 状态 | 优先级 |
|------------|--------------|------|--------|
| lib/protocol | bep-protocol | ✅ 已实现 | - |
| lib/fs | syncthing-fs | ✅ 已实现 | - |
| lib/model | syncthing-sync | ⚠️ 骨架 | P0 |
| lib/connections | syncthing-net | ⚠️ 骨架 | P0 |
| lib/api | syncthing-api | ⚠️ 缺失 | P1 |
| lib/config | syncthing-api/config | ✅ 已实现 | - |
| lib/discover | syncthing-net/discovery | ⚠️ 骨架 | P1 |
| lib/scanner | syncthing-fs/scanner | ✅ 已实现 | - |
| lib/ignore | syncthing-fs/ignore | ✅ 已实现 | - |
| lib/events | syncthing-api/events | ✅ 已实现 | - |
| lib/relay | - | ❌ 缺失 | P2 |
| lib/nat | - | ❌ 缺失 | P2 |
| lib/upnp | - | ❌ 缺失 | P2 |
| lib/stun | - | ❌ 缺失 | P2 |

## 二、关键差距分析

### P0 - 核心阻塞
1. **lib/model** (Go) → **syncthing-sync** (Rust)
   - 差距: 仅有骨架，缺少完整同步状态机
   - 关键: 无法完成端到端同步

2. **lib/connections** (Go) → **syncthing-net** (Rust)  
   - 差距: TCP传输已实现，但连接管理未完整集成
   - 关键: 无法建立可靠的P2P连接

### P1 - 功能缺失
3. **lib/api** (Go REST API)
   - 差距: API端点未实际启动
   - 关键: 无Web界面支持

4. **lib/discover** (Go)
   - 差距: 仅有本地缓存，无全局发现
   - 关键: 设备发现能力弱

### P2 - 增强功能
5. **NAT穿透** (relay/upnp/pmp/stun)
   - 差距: 完全缺失
   - 关键: 公网连接能力

## 三、子代理任务分配

### Agent-Sync (优先级: P0)
**负责**: syncthing-sync 完整实现
**参考**: `syncthing-main/lib/model/`

```
核心文件:
- lib/model/folder.go        → syncthing-sync/src/folder_model.rs
- lib/model/folder_sendrecv.go → 新增 folder_sendrecv.rs
- lib/model/devicer.go       → syncthing-sync/src/model.rs

任务清单:
□ 实现 folder.go 中的扫描/拉取循环
□ 实现冲突解决状态机
□ 集成连接管理器与同步引擎
□ 实现完整 Index/IndexUpdate 处理
```

### Agent-Net (优先级: P0)  
**负责**: syncthing-net 连接管理完善
**参考**: `syncthing-main/lib/connections/`

```
核心文件:
- lib/connections/service.go → syncthing-net/src/manager.rs
- lib/connections/tcp_listen.go → syncthing-net/src/tcp_transport.rs

任务清单:
□ 完善 ConnectionManager 连接池
□ 实现连接优先级管理
□ 集成 TLS 证书轮换
□ 实现设备认证回调
```

### Agent-API (优先级: P1)
**负责**: syncthing-api 服务启动
**参考**: `syncthing-main/lib/api/`

```
核心文件:
- lib/api/api.go            → syncthing-api/src/rest.rs

任务清单:
□ 在 cmd/syncthing 中启动 API 服务
□ 实现 /rest/system/status 端点
□ 实现 /rest/db/status 端点
□ 集成 WebSocket 事件推送
```

### Agent-Discovery (优先级: P1)
**负责**: 全局设备发现
**参考**: `syncthing-main/lib/discover/`

```
任务清单:
□ 实现全局发现客户端
□ 实现 local discovery (multicast)
□ 缓存发现结果
□ 集成到 syncthing-net
```

### Agent-NAT (优先级: P2)
**负责**: NAT穿透实现
**参考**: `syncthing-main/lib/{upnp,pmp,stun,relay}/`

```
任务清单:
□ UPNP 端口映射
□ NAT-PMP/PCP 支持
□ STUN 公网地址发现
□ 中继服务器连接
```

## 四、依赖关系

```
Agent-Sync ←→ Agent-Net (双向依赖)
     ↓
Agent-API (依赖 Sync/Net 状态)
     ↓
Agent-Discovery (被 Net 依赖)
     ↓
Agent-NAT (被 Net 依赖)
```

## 五、验收标准

### Agent-Sync 验收
- [ ] 本地文件变更触发 IndexUpdate
- [ ] 收到远程 Index 后计算差异
- [ ] 实现 pull 循环拉取文件
- [ ] 文件冲突时创建 .sync-conflict 副本

### Agent-Net 验收  
- [ ] 两台实例可建立 TCP+TLS 连接
- [ ] 证书认证成功并提取 DeviceID
- [ ] Hello 消息交换成功
- [ ] 连接断开后自动重连

### Agent-API 验收
- [ ] `curl http://127.0.0.1:8384/rest/health` 返回 200
- [ ] WebSocket 事件流正常推送
- [ ] 浏览器可访问 GUI

## 六、禁止操作

⚠️ **当前阶段不执行**:
- 不修改现有测试代码
- 不重构核心数据结构
- 不添加新依赖（除非必要）

## 七、参考文档

- Go原版: `syncthing-main/lib/{model,connections,api,discover}/`
- BEP协议: `syncthing-main/proto/bep/bep.proto`
- 现有Rust: `syncthing-rust-rearch/crates/*/src/`
