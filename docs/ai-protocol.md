# AI Protocol · syncthing-rust

> 跨架构状态同步锚点。CLI/Web/Claw 会话启动后优先读取。

---

## 接管记录

| 字段 | 值 |
|------|-----|
| 接管时间 | 2026-04-25T22:32+08:00 |
| 前会话 ID | 0ecf987e-f382-4d86-bc56-dd6b89a7c75c |
| 接管原因 | 节奏拖延，多次违背工程项目严谨性 |
| 接管后提交 | 3 commits (0c1dd56, 20659da, 8e4e84e) |

## 工作区状态快照

- **分支**: main，ahead of origin by 9 commits
- **编译**: `cargo build --workspace` ✅ 通过
- **测试**: `cargo test --workspace` ✅ 279 passed, 1 ignored, 0 failed
- **clippy**: `cargo clippy --workspace --all-targets` ✅ 0 warnings
- **工作区**: clean

## 已执行的接管修正

1. `0c1dd56` — 提交滞留的诊断改动（块大小校验、Puller 错误上下文、扫描触发溯源）
2. `20659da` — 修正 AGENTS.md 文档矛盾：Global Discovery & Relay Protocol 标记已完成；归档 4 份过时 Agent 任务模板（Agent-A/B/D/E）
3. `8e4e84e` — 更新 SESSION_STARTER.md 测试数量与阻塞项

## 真实阻塞项

**格雷端 BEP 互通验证**。Go Syncthing 未监听 Tailscale IP (`100.99.240.98:22000`)，Rust 端 dial 被拒绝 (os error 10061)。
- 需格雷确认 Go 节点运行状态及监听地址，或提供替代地址
- **本机不可独立解除**

## 当前粗粒度待办（按优先级）

| 优先级 | 任务 | 类型 | 依赖 |
|--------|------|------|------|
| P0 | 格雷端 BEP 互通验证（完整握手 + 文件同步） | 验证 | 格雷 |
| P1 | BEP 扩展 `Verify` 消息类型草案 | 设计文档 | 无 |
| P1 | 跨实例发现与握手流程图 | 设计文档 | 无 |
| P2 | Local Discovery IPv6 多播 / 网卡枚举 | 代码 | 无 |
| P2 | STUN NAT 类型检测 / hole punching | 代码 | 无 |
| P2 | PortMapper daemon 自动续约（PCP/NAT-PMP） | 代码 | 无 |

## 阶段性冻结项

- 共识算法实现
- 信誉系统
- 加密信道重建

**冻结理由**: 当前阶段（v0.2.0 Beta）核心目标是 P2P 文件同步与官方 Go Syncthing 互操作。上述三项属于 L4 元认知远期扩展，当前投入产出比极低（高实现成本 + 零实际部署场景），待多实例生产验证后再解冻。

## 快速恢复命令

```powershell
cd dev/third_party/syncthing-rust
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

## 信任边界

- 子代理交付物视为不可靠，任何声称"已完成"的功能必须经 `cargo check` + `cargo test` 独立验证
- Rust 核心模块不可外包给子 Agent

---

## 调试经验教训（2026-04-27 大文件传输攻坚）

### 1. Relay Protocol 槽位竞争 → `early eof`
**现象**：listener 反复 `join_relay` 成功，但 `wait_invitation` 立刻 `read header: early eof`。
**根因**：dialer `connect_bep_via_relay` 在 `request_session` 后保持 `RelayProtocolClient`（Protocol Mode TLS）打开，直到 BEP TLS + Hello 完成后才 drop。这期间 listener 的 `join_relay` 返回 `already connected`，relay 服务器随后断开 listener。
**修复**：`request_session` 后立即 `drop(protocol_client)`；health check 的 `join_relay` 后也立即 `drop(client)`。

### 2. Scanner 竞态 → 文件被误删
**现象**：大文件下载到一半，被 scanner 标记为 `deleted`。
**根因**：scanner 扫描时看到 `.syncthing.tmp` 临时文件不在数据库中（因为最终文件名还没 rename），将其视为已删除。
**修复**：扫描时检查 `full_path.with_extension(".syncthing.tmp").exists()`，若存在则跳过删除标记。

### 3. `block_no` 硬编码 → Go 端 `InvalidFile`
**现象**：大文件（>1 block）传输时，Go 端返回 `InvalidFile` (code 3)。
**根因**：`ManagerBlockSource::try_request_block_from_device` 中 `block_no` 硬编码为 `0`。Go 端用 `BlockNo * blockOverhead` 计算加密 padding，block>0 时 padding 错配。
**修复**：传入 `enumerate()` 的实际块索引 `idx`。

### 4. `pending_responses` insert-after-send 竞态
**现象**：BEP Response 被丢弃为 "unmatched response"。
**根因**：`send_message(Request)` 先于 `pending_responses.insert(id, tx)`。快响应在 insert 前到达，找不到匹配 entry。
**修复**：`insert` 必须在 `send_message` 之前。

### 5. `pending_responses` 内存泄漏
**现象**：Session 断开后，`rx.await` 永远阻塞。
**根因**：`BepSession::run` 返回前未清理未完成的 `pending_responses` entry。
**修复**：Session 结束前遍历所有未完成 entry，发送 `Generic` error Response 唤醒等待方。

### 6. Heartbeat 参数与 NAT/relay 稳定性
**现象**：连接空闲 4.5 分钟后断开。
**根因**：默认 heartbeat interval 90s / timeout 270s，在 relay+NAT 场景下 NAT mapping 超时。
**修复**：interval 30s / timeout 600s。

### 7. 诊断效率：日志级别升级优于时序推演
**教训**：在 info 级日志中反复推演 race condition 时序效率极低。当应用层无响应时，应迅速升级到 `--log-level debug`，直接观察 `request_block`、`send_message`、`recv_message` 的每一步行为。

### 8. 日志缓冲陷阱
**教训**：stdout 重定向到文件存在缓冲，`tail` 看到的"连接正常"可能是旧状态。应以对端实际日志（或 `lsof`/进程状态）为准，避免被缓冲延迟误导。

### 9. Race resolution 的潜在 cleanup 竞态（待验证）
**假设**：旧 BepSession 被 `abort()` 后，其 future 中的 `handle.disconnect(&device_id, ...)` 可能在新连接注册后才执行，导致新连接被意外关闭。
**状态**：尚未最终确认，但时序日志显示 12:45:38 连接被关闭后，BepSession 又过了 4 分钟才因 `ping send error` 结束，值得后续深入。

### 快速恢复命令（本机验证）
```powershell
cd dev/third_party/syncthing-rust
cargo build --release --bin syncthing
taskkill /F /IM syncthing.exe
Start-Process .\target\release\syncthing.exe -ArgumentList "run","--config-dir","$env:LOCALAPPDATA\syncthing-rust","--log-level","debug"
```
