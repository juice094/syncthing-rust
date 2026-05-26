---
type: plan
status: active
project: syncthing-rust
date: 2026-05-17
tags: [plan, roadmap]
---

# 后续任务清单 — 2026-05-17

> 承接 [NEXT_STEPS_2026-05-15.md](./NEXT_STEPS_2026-05-15.md)
> 本文档基于 v0.2.9-rc1 维护轮次完成后的状态快照生成。
> **状态快照（2026-05-17 23:59）**：11 commits 已推送至 origin/main，tag `v0.2.9-rc1` 已发布；CI 全绿；Prometheus /metrics、跨平台监控、双节点编排基础设施全部就绪。

---

## 0. 今日关键事件

| 时间 | 事件 | 结论 |
|------|------|------|
| 全天 | v0.2.8 → v0.2.9-rc1 维护轮次 | 代码健康、可观测性、CI、双节点编排四大工作流完成 |
| 晚间 | 双节点编排器首次运行 | 本地 Windows 节点验证通过；远程 Linux 部署包已生成 |
| 晚间 | 远程 Linux 环境确认 | Ubuntu 24.04, cargo 1.95.0, 端口 22001 空闲；GitHub 访问受限 |

---

## 1. 已完成项（自 2026-05-15 以来）

### 1.1 代码健康

| 任务 | 状态 | 提交 |
|------|------|------|
| 超大文件拆分（session/mod.rs 609行 → state.rs） | ✅ | 771e544 |
| 超大文件拆分（connection/mod.rs 606行 → io.rs） | ✅ | 771e544 |
| self_weak helper 提取 + INVARIANT 注释 | ✅ | a1461f3 |
| placeholder 字段标注 TODO(v0.3.0) | ✅ | 3009596 |
| unwrap 生产代码清理 | 🟡 | 396 → 368，持续进行中 |

### 1.2 可观测性（O-1 完成）

| 任务 | 状态 | 提交 |
|------|------|------|
| Prometheus `/metrics` 端点 | ✅ | 26db471 |
| 跨平台 `syncthing-monitor` 二进制 | ✅ | 4c34324 |
| CSV/JSON 输出 + 告警阈值 | ✅ | 4c34324 |

### 1.3 CI 增强

| 任务 | 状态 | 提交 |
|------|------|------|
| cargo-deny | ✅ | f6a6c9e |
| all-features 测试 | ✅ | f6a6c9e |
| macOS 矩阵 | ✅ | f6a6c9e |
| justfile | ✅ | f6a6c9e |

### 1.4 双节点真实网络测试基础设施

| 任务 | 状态 | 提交 |
|------|------|------|
| `gen_test_config` 二进制（类型安全配置生成） | ✅ | 20bb475 |
| `two-node-real-network-test.ps1` 编排器 | ✅ | 18909e8 |
| `stop-two-node-test.ps1` 优雅终止 | ✅ | 18909e8 |
| `check-sync-consistency.ps1` 一致性校验 | ✅ | 18909e8 |
| `churn-files.ps1` 独立文件扰动 | ✅ | 20bb475 |
| 本地 Windows 节点验证 | ✅ | 20bb475 |
| 远程 Linux 部署包生成 | ✅ | 20bb475 |

---

## 2. 立即执行（P0 — 不阻塞不推进）

### T-Net-1b 双节点真实网络 E2E 验证

**目标**：完成首次 Windows ↔ Linux 跨真实网络文件同步。

| 项 | 内容 | 状态 |
|---|---|---|
| 网络 | Tailscale（标准官方控制平面） | ✅ 双方已接入 |
| 本地 | Windows 11 `rog-x` (100.107.247.38) | ✅ 编排器验证通过 |
| 远程 | Ubuntu 24.04 `iv-yekqy2vdhcbw80dkpmoo` (100.127.13.26) | ⏳ 二进制待传输 |
| 阻塞 | 远程 cargo 构建因 GitHub 访问受限失败 | **需本侧交叉编译或 GitHub Actions 构建** |

**下一步动作**：
1. **方案 A（推荐）**：在本侧 Windows 设置 x86_64-unknown-linux-gnu 交叉编译工具链，编译 `syncthing` + `syncthing-monitor` Linux 二进制，通过 scp 传输到远程
2. **方案 B**：使用 GitHub Actions 构建 Linux release 产物，远程 `wget` 下载
3. **方案 C**：在远程手动配置 cargo registry 镜像（清华源 / rsproxy），本地构建

### T-Net-2 网络层抽象 RFC（Transport Plugin）

**目标**：让 syncthing-rust 支持多种底层传输。

| 项 | 内容 |
|---|---|
| 问题 | 当前仅支持 TCP 22001；政企场景防火墙常拦截 |
| 方案 | 抽象 `Transport` trait，支持 tcp / tailscale / wireguard / unix_socket |
| 范围 | `crates/syncthing-net/src/transport/` 新增 proxy transport |
| 预计 | 3d |
| 优先级 | P0 |

---

## 3. v0.3.0 准入标准（更新）

- [x] 所有 channel 必须有界 (H-3)
- [x] 所有 loop 必须可优雅终止 (H-6)
- [x] 生产代码零 panic (H-5)
- [x] 日志轮转通过单日 10GB 压力测试 (H-2)
- [x] §1 ClusterConfig race 修复 (T3.1)
- [x] T3.1b `reconnect_device` API + session health check
- [x] 跨版本互通验证 (Rust v0.2.6 vs Go v2.1.0)
- [x] **Prometheus /metrics 端点可读取连接数、同步速率、错误码** ✅
- [ ] **双节点 BEP E2E 通过真实网络完成文件同步** ← 当前阻塞线
- [ ] **72h 双节点压力测试无内存泄漏、无死锁**
- [ ] **Transport Plugin RFC 合并**

---

## 4. 技术债追踪

| ID | 债务项 | 之前 | 现在 | 目标 |
|----|--------|------|------|------|
| TD-1a | unwrap/expect 生产代码 | 396 | 368 | <50 |
| TD-1b | syncthing-fs scanner/mod.rs | 43 | 43 | <10 |
| TD-1c | syncthing-db kv.rs + store.rs | 82 | 82 | <20 |
| TD-7 | 超大文件 | 2 | 0 | 0 |
| TD-9 | CI 缺口 | 4 | 0 | 0 |

---

## 5. 已知问题

| 编号 | 问题 | 影响 | 缓解措施 |
|------|------|------|----------|
| KI-1 | E2E sync test 忽略（ClusterConfig race） | 并行负载下偶发 | T3.1b health check 已在生产环境生效 |
| KI-2 | Windows 进程名匹配需 `.exe` 后缀 | syncthing-monitor 按名称查找时可能失败 | 当前使用 PID 模式，不受影响 |
| KI-3 | API server 启动间歇性失败 | Windows 特定 | 已在历史 handoff 中记录 |
| KI-4 | 远程 Linux GitHub 访问受限 | 无法 cargo build | 需交叉编译或 CI 产物分发 |

---

## 6. 风险与回退

| 风险 | 触发条件 | 应对 |
|------|----------|------|
| 交叉编译工具链安装失败 | Windows 缺少 Linux 链接器 | 转方案 B（GitHub Actions 构建） |
| Transport Plugin 改动面过大 | 影响现有 TCP/Relay 稳定性 | 拆 v0.3.0（接口）+ v0.3.1（实现） |
| 72h 压测暴露内存泄漏 | RSS 持续增长 | syncthing-monitor 已配置告警阈值，可提前发现 |

---

## 7. 跨文件跳转速查

- **当前权威路线图**：本文件
- **上一版计划**：[NEXT_STEPS_2026-05-15.md](./NEXT_STEPS_2026-05-15.md)
- **维护轮次 handoff**：[../handoffs/HANDOFF_2026-05-17_TWO_NODE_TEST.md](../handoffs/HANDOFF_2026-05-17_TWO_NODE_TEST.md)
- **已知缺陷登记**：[../KNOWN_ISSUES.md](../KNOWN_ISSUES.md)
- **架构路线图**：[POST_V0_2_0_ROADMAP.md](./POST_V0_2_0_ROADMAP.md)

---

**维护人**：juice094
**下次刷新**：双节点 E2E 验证完成 或 v0.3.0 首个 PR 合并时
