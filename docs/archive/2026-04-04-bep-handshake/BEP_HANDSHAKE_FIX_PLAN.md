# BEP Handshake 修复计划

## 问题分析

### 1. 编译错误
- `handshake.rs` 缺少 `exchange_hello` 函数
- `lib.rs` 尝试导出 `exchange_hello` 但函数不存在

### 2. Hello 交换失败原因
从测试日志分析：
```
TLS handshake successful with 100.99.240.98:22000 (device: IKOL33P)
Hello hex dump (45 bytes): 2e a7 d9 0b 00 27 0a 05 52 4f 47 2d 58...
Sent Hello: device=ROG-X client=syncthing-rust/0.1.0
Hello exchange failed: Failed to read Hello magic: 远程主机强迫关闭了一个现有的连接
```

- ✅ TLS 握手成功
- ✅ Hello 格式正确（magic 0x2EA7D90B, length 39, protobuf 正确）
- ❌ 服务端收到 Hello 后立即关闭连接

### 3. 根本原因
对比 Go 原版代码发现：

**Go 原版证书生成** (`lib/tlsutil/tlsutil.go:132`):
```go
DNSNames: []string{commonName},  // 包含 SAN
```

**Rust 证书生成** (`tls.rs`):
- 之前缺少 SAN (Subject Alternative Name)
- Go 1.15+ 要求证书必须有 SAN，否则 TLS 握手后验证失败

但已修复证书 SAN 问题。

**另一个可能原因**：
Go 服务端 `OnHello` 处理逻辑中，如果设备未配置，会添加到待处理列表，但不会拒绝连接。问题可能出在：
1. 服务端期望我们先接收它的 Hello（时序问题）
2. 服务端配置问题（AllowedNetworks 限制）
3. 证书 CN 验证问题

## 修复方案

### 方案 A：修复 exchange_hello 函数（必需）
在 `handshake.rs` 中添加 `exchange_hello` 函数：

```rust
pub async fn exchange_hello<S>(
    stream: &mut S,
    our_hello: &Hello,
) -> Result<Hello>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // 1. 编码 Hello（magic + length + protobuf）
    let hello_bytes = our_hello.encode_with_header()
        .map_err(|e| SyncthingError::Protocol(format!("Failed to encode Hello: {}", e)))?;
    
    // 2. 发送 Hello
    timeout(Duration::from_secs(30), stream.write_all(&hello_bytes)).await
        .map_err(|_| SyncthingError::Timeout("Hello write timeout".to_string()))?
        .map_err(|e| SyncthingError::Network(format!("Failed to send Hello: {}", e)))?;
    
    timeout(Duration::from_secs(30), stream.flush()).await
        .map_err(|_| SyncthingError::Timeout("Hello flush timeout".to_string()))?
        .map_err(|e| SyncthingError::Network(format!("Failed to flush Hello: {}", e)))?;
    
    // 3. 读取对方 Hello（magic + length + protobuf）
    let mut magic_buf = [0u8; 4];
    timeout(Duration::from_secs(30), stream.read_exact(&mut magic_buf)).await
        .map_err(|_| SyncthingError::Timeout("Hello read timeout".to_string()))?
        .map_err(|e| SyncthingError::Network(format!("Failed to read Hello magic: {}", e)))?;
    
    let magic = u32::from_be_bytes(magic_buf);
    if magic != HELLO_MESSAGE_MAGIC {
        return Err(SyncthingError::Protocol(format!("Invalid Hello magic: 0x{:08x}", magic)));
    }
    
    let mut len_buf = [0u8; 2];
    timeout(Duration::from_secs(30), stream.read_exact(&mut len_buf)).await
        .map_err(|_| SyncthingError::Timeout("Hello read timeout".to_string()))?
        .map_err(|e| SyncthingError::Network(format!("Failed to read Hello length: {}", e)))?;
    
    let msg_len = u16::from_be_bytes(len_buf) as usize;
    if msg_len > MAX_HELLO_SIZE {
        return Err(SyncthingError::Protocol("Hello message too big".to_string()));
    }
    
    let mut msg_buf = vec![0u8; msg_len];
    timeout(Duration::from_secs(30), stream.read_exact(&mut msg_buf)).await
        .map_err(|_| SyncthingError::Timeout("Hello read timeout".to_string()))?
        .map_err(|e| SyncthingError::Network(format!("Failed to read Hello body: {}", e)))?;
    
    let remote_hello = Hello::decode(&msg_buf[..])
        .map_err(|e| SyncthingError::Protocol(format!("Failed to decode Hello: {}", e)))?;
    
    Ok(remote_hello)
}
```

### 方案 B：验证证书 SAN（已完成）
确保 `tls.rs` 生成证书时包含 SAN：
```rust
let mut params = CertificateParams::new(vec![common_name.to_string()])?;
```

### 方案 C：添加调试日志（可选）
在关键步骤添加十六进制日志，便于排查问题。

## 执行步骤

1. **立即执行**：在 `handshake.rs` 中添加 `exchange_hello` 函数
2. **验证编译**：`cargo check -p bep-protocol`
3. **构建测试**：`cargo build --release -p syncthing`
4. **云端配合**：格雷检查服务端配置和日志

## 交付标准

- [ ] `cargo check` 无错误
- [ ] `cargo build --release` 成功
- [ ] TLS 握手成功（已有）
- [ ] Hello 交换成功（双向）
- [ ] 设备显示为 "Connected"
