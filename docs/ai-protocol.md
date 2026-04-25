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
