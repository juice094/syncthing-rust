---
type: plan
status: archived
project: syncthing-rust
date: 2026-05-15
tags: [plan, roadmap]
---

# 后续任务清单 — 2026-05-15

> 承接 [NEXT_STEPS_2026-05-14.md](./NEXT_STEPS_2026-05-14.md)（已归档）
> 本文档基于 v0.2.6 发布后首次真实网络双节点测试 + Error-Budget 架构审计生成。
> **状态快照（2026-05-15 23:59）**：v0.2.6 单机空载 12h 稳定；本侧 WSL2 / 对侧 Gray-Cloud 双节点部署完成；校园网防火墙阻断 TCP 22001 出网；Tailscale/Headscale 被确认为最务实穿透方案；Error-Budget 架构审计完成，识别 5 子系统风险敞口。

---

## 0. 今日关键事件

| 时间 | 事件 | 结论 |
|------|------|------|
| 12:00 | Gray-Cloud 12h 压测报告 | 单机空载稳定，H-1~H-6 有效，日志 6KB（对比事故 21GB） |
| 14:00 | 双节点部署启动 | 对侧（云服务器 101.126.54.134）就绪；本侧（WSL2 Ubuntu）就绪 |
| 15:00 | 裸 TCP 探测失败 | `nc -vz 101.126.54.134 22001` 超时；本侧校园网出口防火墙阻断非标准端口 |
| 16:00 | Tailscale/Headscale 方案确认 | 标准 Tailscale（官方控制平面）用于快速验证；Headscale 用于政企私有化；纯 WireGuard 用于极端隔离 |
| 17:00 | Error-Budget 架构审计 | 5 子系统风险量化：网络层 30%（政企场景已耗尽）、协议层 25%、同步层 20%、安全层 15%、可观测性 10%（已耗尽） |

---

## 1. 立即执行（P0 — 不阻塞不推进）

### T-Net-1 Headscale 双节点 BEP E2E 验证

**目标**：绕过校园网防火墙，完成首次真实网络文件同步。

| 项 | 内容 |
|---|---|
| **对侧** | 部署 Headscale Docker，生成 auth key |
| **本侧** | WSL2 `tailscale up --login-server http://<对侧公网IP>:8080` |
| **配置** | 本侧 syncthing device address 改为对侧 Tailscale IP:22001 |
| **验证** | `rest/connections` total >= 1；A->B / B->A 文件同步成功 |
| **预计** | 1h |

### T-Net-2 网络层抽象 RFC（Transport Plugin）

**目标**：让 syncthing-rust 支持多种底层传输，而不仅是 TCP 22001。

| 项 | 内容 |
|---|---|
| **问题** | 当前 `TransportRegistry` 仅注册 `tcp` + `derp`；政企场景下 TCP 22001 常被防火墙拦截 |
| **方案** | 抽象 `Transport` trait，允许通过配置文件切换 `tcp` / `tailscale` / `wireguard` / `unix_socket` |
| **范围** | `crates/syncthing-net/src/transport/` 新增 proxy transport 或通用虚拟内网适配层 |
| **预计** | 3d |
| **优先级** | P0（不解决此问题，政企场景无法部署） |

---

## 2. v0.3.0 规划调整（发布后 2~4 周）

v0.3.0 原有目标保留，新增**可观测性基础设施**和**网络层抽象**为必含项。

### 2.1 新增必含项

| 任务 | 原状态 | 调整 | 理由 |
|------|--------|------|------|
| **O-1 Prometheus Metrics 端点** | 无 | **新增 P0** | Error-Budget 审计判定可观测性 budget 已耗尽；政企环境必须对接 Prometheus/ELK |
| **O-2 Structured Logging** | 无 | **新增 P1** | 当前 tracing 输出为 ANSI 彩色文本，无法被日志系统解析；需增加 JSON formatter |
| **T-Net-2 Transport Plugin** | 无 | **新增 P0** | 校园网/政务网阻断 TCP 22001，必须支持虚拟内网穿透 |
| **T2.4 Linux 72h 压测** | P0 | **调整为双节点 72h** | 单机空载 12h 已验证；v0.3.0 必须完成**双节点真实网络** 72h 压力测试 |

### 2.2 保留项

| 任务 | 状态 |
|------|------|
| T3.1b `connect_to` 重连语义 | 保留 |
| §4 TestNode rescan interval | 保留 |
| T3.4 dialer 业务拆分 | 保留 |

### 2.3 v0.3.0 准入标准（更新）

- [x] 所有 channel 必须有界 (H-3)
- [x] 所有 loop 必须可优雅终止 (H-6)
- [x] 生产代码零 panic (H-5)
- [x] 日志轮转通过单日 10GB 压力测试 (H-2)
- [x] §1 ClusterConfig race 修复 (T3.1)
- [x] T3.1b `reconnect_device` API + session health check
- [x] 跨版本互通验证 (Rust v0.2.6 vs Go v2.1.0)
- [ ] **双节点 BEP E2E 通过真实网络完成文件同步** <- 新增准入线
- [ ] **Prometheus /metrics 端点可读取连接数、同步速率、错误码** <- 新增准入线
- [ ] **72h 双节点压力测试无内存泄漏、无死锁** <- 替代原单机 72h
- [ ] **Transport Plugin RFC 合并** <- 新增准入线

---

## 3. v0.4.0 政企合规路线图（新立项）

v0.4.0 不追求功能增加，追求**政企生产准入**。这是一个独立里程碑，可能需要 4~6 周。

### 3.1 密码合规

| 任务 | 内容 | 工作量 |
|------|------|--------|
| C-1 国密 TLS RFC | 评估 rustls + ring 替换为支持 SM2/SM3/SM4 的密码库可行性 | 1w |
| C-2 证书外部化 | 支持从文件系统挂载 PKCS#12 / PEM 证书，替代自生成 Ed25519 | 3d |
| C-3 证书白名单 | Config.devices 增加 cert_fingerprint 字段，强制校验对端证书指纹 | 2d |

### 3.2 存储与一致性

| 任务 | 内容 | 工作量 |
|------|------|--------|
| C-4 FileSystemDatabase -> SQLite WAL | 替换 JSON 文件存储，支持事务、索引、崩溃恢复 | 2~3w |
| C-5 Scanner 并行化 | SHA-256 计算改为 rayon 并行，降低大文件扫描延迟 | 1w |

### 3.3 审计与可观测性

| 任务 | 内容 | 工作量 |
|------|------|--------|
| C-6 审计日志接口 | 所有配置变更、设备连接/断开、文件同步事件写入结构化审计日志 | 1w |
| C-7 OpenTelemetry Tracing | 替换纯 tracing，支持 OTLP 导出到 Jaeger/SkyWalking | 1w |

### 3.4 代码健壮性

| 任务 | 内容 | 工作量 |
|------|------|--------|
| C-8 系统消除 unwrap | 生产代码 unwrap 从 337 降至 <50；引入 cargo fuzz 进行模糊测试 | 持续 |
| C-9 安全审计 | 聘请第三方或自行进行渗透测试（focus: TLS 握手、REST API 鉴权、config 热重载） | 1w |

---

## 4. Error-Budget 架构审计摘要

基于 error-budget-engineering 框架，各子系统风险敞口如下：

```
网络传输层    ████████████████████████████████  30%  [政企场景已耗尽]
协议握手层    ██████████████████████████        25%  [国密缺失]
数据同步层    ██████████████████████            20%  [DB 瓶颈]
运行时安全    ███████████████                   15%  [unwrap 337]
可观测性      ██████████                        10%  [完全缺失]
```

**判决**：当前 v0.2.6 适合 **受控实验、技术验证、个人私有部署**；**政企生产准入未通过**。

详细审计报告：见 `docs/audit/ARCHITECTURE_AUDIT_2026-05-15.md`（待生成）。

---

## 5. 工程纪律更新（自 2026-05-15 起生效）

在 2026-05-14 五项纪律基础上新增：

6. **网络层必须有 fallback**：任何硬编码 `0.0.0.0:22001` 的代码必须允许通过配置覆盖，且必须支持虚拟内网 IP（Tailscale/WireGuard）作为 device address。
7. **新增代码必须有可观测性**：新模块必须暴露至少 1 个 Prometheus-style counter/gauge（即使只是 `/metrics` 文本输出）。
8. **政企场景优先原则**：当个人场景与政企场景冲突时（如端口选择、密码算法），默认走政企兼容路径。

---

## 6. 风险与回退

| 风险 | 触发条件 | 应对 |
|------|----------|------|
| Headscale 部署阻塞 | 云服务器 Docker 不可用或 UDP/443 被拦 | 退至纯 WireGuard 手动配置 |
| Transport Plugin 改动面过大 | 影响现有 TCP 和 Relay 稳定性 | 拆分为 v0.3.0（接口抽象）+ v0.3.1（Tailscale 实现） |
| Prometheus 引入新依赖 | prometheus crate 增加编译时间和二进制体积 | 使用最简单的文本格式实现（`/metrics` 返回 plain text），不引入 heavy client |
| 国密改造阻塞 | rustls/ring 不支持 SM2/SM3 | 延至 v0.4.1，先完成证书外部化（C-2）作为过渡 |

---

## 7. 跨文件跳转速查

- **当前权威路线图**：本文件
- **昨日计划**：[NEXT_STEPS_2026-05-14.md](./NEXT_STEPS_2026-05-14.md)
- **已知缺陷登记**：[../KNOWN_ISSUES.md](../KNOWN_ISSUES.md)
- **调优母计划**：[TUNING_PLAN_2026-05-11.md](./TUNING_PLAN_2026-05-11.md)
- **架构路线图**：[POST_V0_2_0_ROADMAP.md](./POST_V0_2_0_ROADMAP.md)
- **架构审计**：[../audit/ARCHITECTURE_AUDIT_2026-05-15.md](../audit/ARCHITECTURE_AUDIT_2026-05-15.md)（待生成）

---

## 附录 A：双节点部署状态（2026-05-15）

| 角色 | 环境 | Device ID | 状态 |
|------|------|-----------|------|
| 本侧 | WSL2 Ubuntu | `GHSF54R-HI7K3X4-QP4ZOBE-LPVZC6E-JT7LVJI-NDHLUO7-6Z3GE7Q-Q5CBDQ4` | 配置完成，等待穿透 |
| 对侧 | Gray-Cloud Linux | `IDF323Z-KWINU57-LFA6465-ZUFRF3R-WFA4PKK-3YHQ2JP-IWCGDND-NSWACAA` | 监听 0.0.0.0:22001，配置完成 |
| **阻塞** | 校园网出口防火墙 | — | TCP 22001 出网被拦截 |

---

**维护人**：juice094
**下次刷新**：Headscale E2E 验证完成 或 v0.3.0 首个 PR 合并时
