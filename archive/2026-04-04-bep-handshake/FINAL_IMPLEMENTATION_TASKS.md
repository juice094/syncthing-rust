# 最终实现任务分配

> 目标：将项目完成度从 55-60% 提升到 75-80%

---

## 任务1: Agent-Integration - API服务启动

**目标**: 在 `syncthing run` 时实际启动 REST API 服务

**当前状态**: 代码存在但未调用
**验收标准**: `curl http://127.0.0.1:8384/rest/health` 返回 200

### 需要修改的文件

1. `cmd/syncthing/src/main.rs`
   - 创建 `ApiState` 实例
   - 创建 `RestApi` 实例
   - 使用 `tokio::spawn` 启动 API 服务
   - 处理 graceful shutdown

### 关键代码模板

```rust
// 在 cmd_run 函数中
use syncthing_api::{EventBus, RestApi, ApiState};

// 创建事件总线
let event_bus = EventBus::new();

// 创建 API 状态
let api_state = ApiState::new(
    store.clone(),
    event_bus,
    Some(sync_service.sync_engine()),
);

// 创建并启动 API
let mut rest_api = RestApi::new(api_state);
rest_api.bind(gui_address.parse()?).await?;
let api_handle = tokio::spawn(async move {
    rest_api.run().await
});

// 在 shutdown 时
api_handle.abort();
```

---

## 任务2: Agent-E2E - 端到端同步集成

**目标**: 完成 scan_loop 和 pull_loop 的完整集成

**当前状态**: 骨架代码存在但未集成到 SyncService
**验收标准**: 文件变更后能触发同步流程

### 需要完成的工作

1. `syncthing-sync/src/service.rs`
   - 在 `start()` 中启动 scan_loop
   - 在 `start()` 中启动 pull_loop
   - 将 loop 与 FolderModel 关联

2. `syncthing-sync/src/folder_model.rs`
   - 完善 FolderModel::run() 方法
   - 实现 scan -> diff -> pull 流程

3. `syncthing-sync/src/model.rs`
   - SyncEngine 处理远程 Index
   - 触发 pull 调度

### 关键流程

```
文件变更
  ↓
scan_loop 检测
  ↓
生成 LocalIndexUpdated 事件
  ↓
IndexHandler 处理
  ↓
发送 IndexUpdate 到远程设备
  ↓
远程设备接收
  ↓
pull_loop 检测到需要同步
  ↓
Puller 下载文件块
  ↓
文件写入完成
```

---

## 任务3: Agent-Interop - 与Go原版互通测试

**目标**: 验证与 Go Syncthing 的兼容性

**当前状态**: 未测试
**验收标准**: 能与 Go Syncthing 建立连接并交换索引

### 测试步骤

1. **编译 Go 原版**
   ```bash
   cd syncthing-main
   go run ./cmd/syncthing
   ```

2. **启动 Rust 版本**
   ```bash
   cd syncthing-rust-rearch
   ./target/release/syncthing.exe run
   ```

3. **验证连接**
   - TCP 22000 端口连接建立
   - TLS 握手成功
   - Hello 消息交换成功
   - ClusterConfig 交换成功
   - Index 交换成功

### 需要调试的问题

- 证书格式兼容性
- 协议版本协商
- 消息编码差异

---

## 任务4: Agent-NAT - NAT穿透实现

**目标**: 实现基本的 NAT 穿透功能

**当前状态**: 完全缺失
**验收标准**: 公网环境下的设备发现

### 实现优先级

1. **STUN** (最高优先级)
   - 获取公网地址
   - 参考: `syncthing-main/lib/stun/`

2. **UPnP** (中等优先级)
   - 端口自动映射
   - 参考: `syncthing-main/lib/upnp/`

3. **Relay** (最低优先级)
   - 中继服务器连接
   - 参考: `syncthing-main/lib/relay/`

### 实现文件

- `crates/syncthing-net/src/stun.rs` (新建)
- `crates/syncthing-net/src/upnp.rs` (新建)
- `crates/syncthing-net/src/relay.rs` (新建)

---

## 验收标准汇总

### Agent-Integration 验收
```bash
# 编译通过
cargo build --release

# 启动服务
./syncthing.exe run &

# API测试
curl http://127.0.0.1:8384/rest/health
curl http://127.0.0.1:8384/rest/system/status
```

### Agent-E2E 验收
```bash
# 创建测试文件夹
mkdir -p ~/syncthing-test
./syncthing.exe folder add test ~/syncthing-test

# 添加文件
echo "test" > ~/syncthing-test/file.txt

# 验证扫描触发（查看日志）
# 验证索引生成
```

### Agent-Interop 验收
```bash
# 启动Go原版和Rust版本
# 互相添加设备
# 验证连接建立（查看日志中的设备连接消息）
```

### Agent-NAT 验收
```bash
# STUN测试
cargo test -p syncthing-net stun

# UPNP测试（需要路由器支持）
cargo test -p syncthing-net upnp
```

---

## 任务依赖关系

```
Agent-Integration (API启动)
        ↓
Agent-E2E (同步集成) ←→ Agent-Interop (互通测试)
        ↓
Agent-NAT (NAT穿透)
```

**并行策略**:
- Agent-Integration 和 Agent-NAT 可以并行
- Agent-E2E 和 Agent-Interop 需要部分串行

---

## 禁止事项

❌ **不允许**:
1. 修改现有测试代码来通过测试
2. 破坏现有编译通过的代码
3. 添加大量unsafe代码
4. 引入不必要的外部依赖
5. 改变现有架构设计

✅ **允许**:
1. 添加新模块和新文件
2. 修改 main.rs 集成新功能
3. 修复编译错误
4. 添加新测试
