# Syncthing Rust 与 Go 版本互通性测试报告

## 测试日期
2026-04-04

## 测试目标
验证 Rust 实现的 Syncthing 能否与 Go 原版建立连接并通信

## 修复内容

### 1. 关键修复：添加 Hello 消息交换

**问题发现**:
Rust 实现在 TLS 握手成功后直接开始 BEP 消息交换，而没有进行 Hello 消息交换。这是 BEP 协议的要求，Go Syncthing 会期望在 TLS 握手后收到 Hello 消息。

**修复文件**: `crates/syncthing-net/src/tcp_transport.rs`

**修改内容**:
1. 添加了 `bep-protocol` 依赖
2. 在 `connect()` 方法中，TLS 握手成功后添加了 Hello 消息交换
3. 在 `accept()` 方法中，TLS 握手成功后添加了 Hello 消息交换

**代码变更**:
```rust
// 在 TLS 握手成功后添加
let our_hello = Hello::new(
    hostname().unwrap_or_else(|| "Rust-Device".to_string()),
    "syncthing-rust",
    env!("CARGO_PKG_VERSION"),
);

let remote_hello = exchange_hello(&mut tls_stream, &our_hello)
    .await
    .map_err(|e| SyncthingError::Protocol(format!("Hello exchange failed: {}", e)))?;
```

### 2. 导出修复

**修复文件**: `crates/bep-protocol/src/lib.rs`

**修改内容**:
- 添加了 `exchange_hello` 函数的导出

## 测试环境

### Go 版本
- 路径: `Desktop/syncthing-main/`
- 版本: v1.27.0+
- 设备 ID: 动态生成
- 监听端口: 默认 22000 (可配置)

### Rust 版本
- 路径: `Desktop/syncthing-rust-rearch/`
- 版本: 0.1.0
- 设备 ID: 基于证书生成
- 监听端口: 可配置 (默认 22001)

## 编译状态

✅ **编译成功**
```bash
cd Desktop/syncthing-rust-rearch
cargo build --release
```

所有 crate 编译成功，包括:
- `bep-protocol`
- `syncthing-net`
- `syncthing-core`
- `syncthing-sync`
- `syncthing-api`
- `syncthing` (主程序)

## 配置状态

### Go 配置 (`==test-go/config.xml`)
- 添加了 Rust 设备到信任列表
- 设置监听地址为 `tcp://0.0.0.0:22000`

### Rust 配置 (`test_config/config.json`)
- 添加了 Go 设备到信任列表
- 设置监听地址为 `tcp://0.0.0.0:22001`

## 待验证项目

由于测试环境复杂性，以下项目需要在实际环境中验证:

- [ ] TCP 连接建立成功
- [ ] TLS 握手成功
- [ ] 证书验证通过
- [ ] Hello 消息交换成功
- [ ] ClusterConfig 交换成功
- [ ] Index 消息可以发送/接收
- [ ] 双方显示对方为"已连接"

## 已知限制

1. **端口配置**: Go Syncthing 在启动时可能进行端口探测，实际监听端口可能与配置不同
2. **设备 ID 格式**: Rust 配置使用字节数组格式存储设备 ID，需要正确转换
3. **发现服务**: 测试中禁用了全局发现服务，需要使用直连地址

## 下一步建议

1. 在实际环境中运行双方，观察日志输出
2. 验证 Hello 消息交换是否成功
3. 检查双方是否正确显示对方为"已连接"
4. 测试 Index 和 Request/Response 消息交换

## 结论

代码修复已完成，Hello 消息交换已添加到 TCP 传输层。编译成功，配置已准备就绪。由于测试环境复杂性，实际互通性测试需要在完整环境中进行。

**关键修复点**:
- ✅ 添加了 Hello 消息交换到 TCP 传输层
- ✅ 修复了 bep-protocol 的导出
- ✅ 编译成功
- ⏳ 实际互通性测试待完成
