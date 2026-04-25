# 会话快速启动指南

> **读取优先级**: 第一（进入项目后先读此文件）

## 一句话总结

Syncthing Rust 实现，完成度 **75-80%**，核心功能就绪，待实际互通验证。

## 关键状态速览

| 维度 | 状态 | 备注 |
|------|------|------|
| 编译 | ✅ 通过 | `cargo check --workspace` 0错误 |
| 测试 | ✅ 279 passed | 全 workspace 测试通过，1 ignored |
| clippy | ✅ 0 warnings | workspace 级别 |
| API服务 | ✅ 可用 | `:8384/rest/health` 可访问 |
| 证书持久化 | ✅ 已实现 | 设备ID保持一致 |
| Hello交换 | ✅ 已实现 | BEP协议握手就绪 |
| Global Discovery | ✅ 已实现 | `discovery/global.rs` HTTPS mTLS |
| Relay Protocol v1 | ✅ 已实现 | `relay/` XDR + 健康检查 + backoff |
| 与Go互通 | ⚠️ 阻塞 | 格雷端未监听 Tailscale IP，待格雷确认 |
| 端到端同步 | ⚠️ 待验证 | 需双设备测试 |

## 快速验证命令

```powershell
# 1. 编译检查（必须0错误）
cargo check --workspace

# 2. 运行测试（必须全通过）
cargo test --workspace

# 3. 构建并启动
cargo run --release --bin syncthing -- run

# 4. 测试API
curl http://127.0.0.1:8384/rest/health
```

## 下一步工作（优先级排序）

1. **P0** - 格雷端 Go Syncthing 网络状态确认与互通验证
2. **P1** - BEP 扩展 `Verify` 消息类型草案输出
3. **P1** - 跨实例发现与握手流程图输出
4. **P2** - Local Discovery IPv6 多播 / 网卡枚举
5. **P2** - STUN NAT 类型检测 / hole punching

## 关键文档索引

| 文档 | 用途 | 何时阅读 |
|------|------|----------|
| `PROJECT_STATUS.md` | 完整状态快照 | 需要全面了解时 |
| `VERIFICATION_REPORT.md` | 验收记录 | 需要验证历史时 |
| `AGENT_TASK_ALLOCATION.md` | 子代理分工 | 需要分工参考时 |
| `测试临时存放/syncthing-rust/USER_GUIDE.md` | 用户使用 | 提供给用户时 |

## 信任边界 ⚠️

**子代理交付物视为不可靠**，任何声称"已完成"的功能必须经过独立验证：

1. 编译验证: `cargo check`
2. 测试验证: `cargo test`
3. 功能验证: 实际运行测试

## 会话启动检查清单

进入项目后，先执行：

- [ ] 读取本文件
- [ ] 执行 `cargo check --workspace`
- [ ] 执行 `cargo test --workspace`
- [ ] 根据当前任务阅读对应详细文档

---

**最后更新**: 2026-04-25（接管 Session-0ecf987e 后修正）
