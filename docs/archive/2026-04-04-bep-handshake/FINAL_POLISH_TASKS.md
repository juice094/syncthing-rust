# 项目完善最终阶段任务分配

> 目标：解决阻塞问题，实现与Go原版互通，完成端到端验证

---

## 任务1: Agent-Cert - 证书持久化

**问题**: 每次启动生成新证书，导致设备ID不断变化

**影响**: 
- Go原版拒绝连接（设备ID不匹配）
- 无法建立稳定的设备身份

**解决方案**:

1. **证书存储路径**
   ```
   ~/.local/share/syncthing-rust/cert.pem
   ~/.local/share/syncthing-rust/key.pem
   ```

2. **加载逻辑**
   ```rust
   pub fn load_or_generate_certificate(config_dir: &Path) -> Result<DeviceCertificate> {
       let cert_path = config_dir.join("cert.pem");
       let key_path = config_dir.join("key.pem");
       
       if cert_path.exists() && key_path.exists() {
           // 加载现有证书
           load_certificate(&cert_path, &key_path)
       } else {
           // 生成新证书并保存
           let cert = generate_certificate();
           save_certificate(&cert, &cert_path, &key_path)?;
           Ok(cert)
       }
   }
   ```

3. **修改位置**
   - `syncthing-net/src/tls.rs`: `DeviceCertificate::generate()`
   - `cmd/syncthing/src/main.rs`: 证书加载调用

**验收标准**:
```bash
# 第一次启动生成证书
./syncthing.exe run
# 记录设备ID: XXXXXXX-XXXXXXX-...

# 停止后重新启动
./syncthing.exe run
# 设备ID应与第一次相同 ✅
```

---

## 任务2: Agent-Hello - Hello消息交换

**问题**: BEP协议的Hello消息交换不完整

**参考**: `syncthing-main/lib/protocol/bep_hello.go`

**Hello消息格式**:
```
Magic (4 bytes): 0x2EA7D90B
Length (2 bytes): protobuf message length
Message: protobuf encoded Hello
```

**Hello结构**:
```protobuf
message Hello {
    string device_name = 1;
    string client_name = 2;
    string client_version = 3;
    int32 num_connections = 4;
    int64 timestamp = 5;
}
```

**实现要求**:

1. **发送Hello**
   ```rust
   // bep-protocol/src/handshake.rs
   pub async fn send_hello<W: AsyncWrite>(writer: &mut W, hello: &Hello) -> Result<()> {
       writer.write_u32(HELLO_MAGIC).await?;
       let buf = hello.encode_to_vec();
       writer.write_u16(buf.len() as u16).await?;
       writer.write_all(&buf).await?;
       writer.flush().await?;
       Ok(())
   }
   ```

2. **接收Hello**
   ```rust
   pub async fn recv_hello<R: AsyncRead>(reader: &mut R) -> Result<Hello> {
       let magic = reader.read_u32().await?;
       if magic != HELLO_MAGIC {
           return Err(ProtocolError::InvalidMagic);
       }
       let len = reader.read_u16().await?;
       let mut buf = vec![0u8; len as usize];
       reader.read_exact(&mut buf).await?;
       Hello::decode(&buf).map_err(|e| e.into())
   }
   ```

3. **集成到连接建立流程**
   ```rust
   // 在 TCP 连接建立后、TLS 握手后
   async fn establish_connection(stream: TcpStream) -> Result<BepConnection> {
       // 1. TLS 握手
       let tls_stream = tls_handshake(stream).await?;
       
       // 2. Hello 交换
       let hello = Hello {
           device_name: "MyDevice".to_string(),
           client_name: "syncthing-rust".to_string(),
           client_version: "0.1.0".to_string(),
           num_connections: 1,
           timestamp: now(),
       };
       send_hello(&mut tls_stream, &hello).await?;
       let remote_hello = recv_hello(&mut tls_stream).await?;
       
       // 3. 返回连接
       Ok(BepConnection::new(tls_stream, remote_hello))
   }
   ```

**验收标准**:
```bash
# 使用 tcpdump 或日志验证
# 应能看到 Hello 消息交换日志
INFO 发送 Hello: device=MyDevice client=syncthing-rust/0.1.0
INFO 接收 Hello: device=RemoteDevice client=syncthing/1.27.0
```

---

## 任务3: Agent-Interop - 修复互通性

**目标**: 使 Rust 实现能与 Go Syncthing 建立连接

**依赖**: Agent-Cert 和 Agent-Hello 完成后进行

**测试步骤**:

1. **准备测试环境**
   ```bash
   # 终端1: 启动Go原版
   cd syncthing-main
   go run ./cmd/syncthing -home=./test-go -gui-address=127.0.0.1:8386
   # 记录Go设备ID
   
   # 终端2: 启动Rust版本
   cd syncthing-rust-rearch
   ./target/release/syncthing.exe run
   ```

2. **配置互相信任**
   - Go配置: 添加Rust设备ID
   - Rust配置: 添加Go设备ID

3. **验证连接建立**
   - TCP连接建立
   - TLS握手成功
   - Hello交换成功
   - ClusterConfig交换成功

**调试方法**:
- 开启详细日志: `RUST_LOG=debug`
- Go端日志: `STTRACE=protocol,connections`
- Wireshark抓包分析

**验收标准**:
```
Go日志: 连接来自 "rust-device"
Rust日志: 连接到 "go-device"
双方状态: 显示对方为 "已连接"
```

---

## 任务4: Agent-E2ETest - 端到端同步验证

**目标**: 验证两台实例间的实际文件同步

**测试场景**:

### 场景1: 单文件同步
```bash
# 设备A (Go)
echo "test content" > ~/sync-folder/test.txt

# 等待30秒

# 设备B (Rust)
cat ~/sync-folder/test.txt
# 应显示 "test content" ✅
```

### 场景2: 冲突解决
```bash
# 同时修改同一文件
echo "A" > ~/sync-folder/conflict.txt  # 设备A
echo "B" > ~/sync-folder/conflict.txt  # 设备B

# 等待同步

# 应生成冲突文件
ls ~/sync-folder/
# conflict.txt
# conflict.sync-conflict-YYYYMMDD-HHMMSS.txt ✅
```

### 场景3: 大文件同步
```bash
# 生成100MB文件
dd if=/dev/urandom of=~/sync-folder/large.bin bs=1M count=100

# 验证哈希
sha256sum ~/sync-folder/large.bin
# 两台设备哈希应相同 ✅
```

**验收标准**:
- [ ] 文件创建同步成功
- [ ] 文件修改同步成功
- [ ] 文件删除同步成功
- [ ] 冲突正确检测和解决
- [ ] 同步进度正确显示

---

## 任务依赖与并行策略

```
Agent-Cert (证书持久化)
    ↓
Agent-Hello (Hello消息)
    ↓
Agent-Interop (互通测试) ←→ Agent-E2ETest (端对端验证)
```

**并行策略**:
- Agent-Cert 和 Agent-Hello 可以并行
- Agent-Interop 依赖前两个
- Agent-E2ETest 依赖 Agent-Interop

---

## 验收检查清单

### Agent-Cert 验收
```bash
./syncthing.exe run &
ID1=$(grep "设备ID" log.txt | head -1)
pkill syncthing
./syncthing.exe run &
ID2=$(grep "设备ID" log.txt | head -1)
[ "$ID1" = "$ID2" ] && echo "✅ 证书持久化成功"
```

### Agent-Hello 验收
```bash
# 日志中应出现
grep "Hello" log.txt
# Hello 交换成功 ✅
```

### Agent-Interop 验收
```bash
# Go端日志
grep "Connected to" go-log.txt
# 显示 Rust 设备ID ✅
```

### Agent-E2ETest 验收
```bash
# 文件同步测试
[ -f ~/sync-folder/test.txt ] && echo "✅ 同步成功"
```

---

## 预期成果

完成此阶段后，项目将达到：

| 指标 | 当前 | 目标 |
|------|------|------|
| 完成度 | 65-70% | 80-85% |
| API服务 | ✅ | ✅ |
| NAT穿透 | ✅ | ✅ |
| 原版互通 | ⚠️ | ✅ |
| 端到端同步 | ⚠️ | ✅ |
| 生产可用 | ❌ | ⚠️ (接近) |
