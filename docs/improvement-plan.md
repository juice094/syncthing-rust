# Syncthing-rust 项目改进计划 v0.2.0

## 项目现状

- 版本: v0.1.0 (Alpha)
- 代码规模: ~27,500 行 Rust
- 核心功能: Rust ↔ Go 双向文件同步已验证
- 测试: 257 passed, 0 failed
- 架构: 身份层/传输层/网络层三阶段解耦完成

## 改进目标

将 v0.1.0 (Alpha) 推进到 v0.2.0 (Beta)，核心标准:
- 72h 压测无崩溃、无内存泄漏
- 零 panic/unwrap 生产路径
- REST API 与 Go Syncthing 完全兼容
- 连接稳定性达到"set and forget"级别

---

## 工作流 A: 安全加固 (Security) — P0

### A1. 依赖安全扫描
- [ ] 运行 `cargo audit`，修复已知漏洞
- [ ] 检查是否有未使用的依赖（bloat）
- [ ] 评估关键依赖的维护状态（tokio、rustls、axum）

### A2. Panic/unwrap 清理
- [ ] 统计所有 `.unwrap()` / `.expect()` / `panic!`
- [ ] 生产路径（非 test）的 unwrap 全部替换为错误处理
- [ ] 保留 test 中的 unwrap（简化测试）

### A3. 敏感信息保护
- [ ] 检查私钥是否写入日志
- [ ] API key 生成强度（当前 32 字符随机，评估是否足够）
- [ ] TLS 证书文件权限（Windows: 是否其他用户可读？）

---

## 工作流 B: 稳定性加固 (Stability) — P0

### B1. 连接管理器强化
- [ ] 心跳超时后优雅关闭（当前可能直接 abort）
- [ ] 连接质量评分动态更新（RTT、丢包率）
- [ ] 多路径同时保持时的心跳协调
- [ ] 网络变更事件（WiFi 切换）的检测与快速恢复

### B2. 资源泄漏防护
- [ ] 文件句柄泄漏检查（特别是临时文件）
- [ ] TCP 连接泄漏（DerpPipe drop 逻辑验证）
- [ ] mpsc channel 泄漏（sender 未正确关闭的场景）
- [ ] DashMap 条目泄漏（connections/pending 未清理）

### B3. 错误处理一致性
- [ ] 统一错误类型（减少 `SyncthingError::internal(format!("..."))` 的滥用）
- [ ] 错误日志分级（ERROR 只用于需要人工干预的问题）
- [ ] 用户可见错误信息本地化/友好化

---

## 工作流 C: 功能完善 (Features) — P1

### C1. REST API 补齐
- [ ] `/rest/system/connections` ✅ 已完成
- [ ] `/rest/system/log` — 返回运行时日志
- [ ] `/rest/system/upgrade` — 检查更新（返回当前版本）
- [ ] `/rest/system/config` — 完整配置读写
- [ ] `/rest/events` — 事件流（WebSocket/SSE）
- [ ] `/rest/db/file` — 单个文件状态查询

### C2. TUI 配置热同步
- [ ] REST API 修改配置后，TUI 自动重载
- [ ] TUI 修改配置后，REST API 立即生效
- [ ] 文件系统监听 config.json，外部修改自动加载

### C3. 日志与监控
- [ ] 结构化日志输出（JSON 格式选项）
- [ ] 关键指标暴露（Prometheus /metrics 端点）
- [ ] 连接质量历史记录（per-device RTT 趋势）

---

## 工作流 D: 架构演进 (Architecture) — P2

### D1. MagicSocket 抽象
- [ ] 设计 `MagicSocket` trait：统一 direct/relay/ICE 路径
- [ ] `MagicSocket::dial(device_id)` → 自动尝试 direct → ICE → DERP
- [ ] 路径质量实时监控和自动切换

### D2. DERP 自动回退
- [ ] `ParallelDialer` 在 direct 失败后自动尝试 DERP
- [ ] DERP 服务器地址配置（GUI/CLI/config）
- [ ] DERP 路径质量评分（比 direct 差，但可用）

### D3. QUIC 预留
- [ ] 设计 `QuicTransport` 接口（基于 `quinn`）
- [ ] 0-RTT 连接建立
- [ ] NAT 穿透友好的 UDP 打洞

---

## 工作流 E: 测试覆盖 (Testing) — P1

### E1. 集成测试
- [ ] `syncthing-net` ↔ `syncthing-sync` 集成测试
- [ ] REST API 端到端测试（使用 `tokio::test` + `reqwest`）
- [ ] TUI 事件循环测试（模拟键盘输入）

### E2. E2E 测试场景
- [ ] 两个 Rust 节点互相同步（无需 Go 节点）
- [ ] 大文件同步（> 100MB）
- [ ] 大量小文件同步（10,000 个文件）
- [ ] 断网恢复场景

### E3. 性能基准
- [ ] 文件扫描速度基准
- [ ] 网络传输吞吐量基准
- [ ] 内存占用基准

---

## 工作流 F: 生产就绪 (Production) — P2

### F1. 二进制打包
- [ ] Windows: `cargo wix` MSI 安装包
- [ ] Linux: `.deb` / `.rpm` 包
- [ ] 静态链接（musl）二进制

### F2. 服务化
- [ ] systemd service 文件
- [ ] Windows Service 包装
- [ ] 后台守护进程模式（`--daemon`）

### F3. 运维文档
- [ ] 部署指南
- [ ] 监控告警配置
- [ ] 故障排查手册

---

## 执行优先级

```
P0 (立即开始): A1, A2, B1, B2
P1 (本周内):   C1, C2, E1, E2
P2 (下周):     D1, D2, F1, F2
P3 (未来):     D3, F3, C3
```

## 成功标准 (v0.2.0 Exit Criteria)

1. `cargo audit` 零漏洞
2. 生产代码零 unwrap
3. 72h 压测通过（格雷验证）
4. REST API 与 Go Syncthing GUI 完全兼容
5. 连接稳定性：断开后 5 分钟内自动恢复
6. Release build 零警告
