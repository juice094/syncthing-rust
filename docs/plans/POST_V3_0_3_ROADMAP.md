---
type: plan
status: active
project: syncthing-rust
tags: [plan, roadmap, post-v3.0.3]
---

# Post-v3.0.4 后续计划书

> **制定日期**：2026-06-15 · **更新**：2026-06-29 (v3.0.4 已发布)
> **维护者**：juice094
> **制定原则**：风险驱动 + 单人维护约束 + 已冻结项不反复讨论
>
> 本计划承接 [`POST_V0_2_0_ROADMAP.md`](../archive/plans/POST_V0_2_0_ROADMAP.md)（已归档）与 [`CHANGELOG.md`](../../CHANGELOG.md)，聚焦 v3.0.4 发布后的剩余缺口与长期维护方向。

---

## 〇、v3.0.4 已完成的原计划项

v3.0.4 (2026-06-27) 系统性安全审计驱动的版本，超出原 v3.0.x 路线图范围：

| 完成项 | 原计划 | 实际 |
|:---|:---|:---|
| 安全审计 (8 CRITICAL/HIGH 修复) | 未计划 | ✅ 路径穿越/SSRF/连接限流/密钥权限/内容泄露/quinn-proto/级联删除阈值 |
| RBAC 只读 API key | P2 监控增强 | ✅ admin + ro_key |
| Prometheus 指标 + Grafana | P2 监控增强 | ✅ 7 指标 + 9-panel + 7 告警规则 |
| JSON 结构化日志 | 未计划 | ✅ --log-format json (ELK/Splunk/Loki) |
| Relay Server v1 | 部分已有 (client only) | ✅ 完整 Server (Protocol + Session + 转发) |
| WSS 传输 | 未计划 | ✅ TLS-over-WebSocket |
| Docker/K8s 部署 | 未计划 | ✅ Docker + compose + Helm |
| 灾备手册 | 未计划 | ✅ DR runbook |
| SBOM | 未计划 | ✅ CycloneDX 脚本 |
| 测试基线 | 392 → 414 | ✅ +22 tests |

---

## 一、当前状态快照

| 维度 | 状态 |
|:---|:---|
| 核心功能 | Production：BEP/TLS、块级同步、发现、**Relay Server v1**、WSS、REST API (RBAC)、TUI、Windows 托盘均稳定 |
| 安全基线 | **v3.0.4**: 路径穿越/SSRF/连接限流/密钥权限/内容泄露全部修复 |
| 测试基线 | **433 passed / 6 ignored / 0 failed**（2026-07-21 实测） |
| 性能基线 | Puller 6.7GiB/s, Scanner 803µs/MB, BEP 198ns/Hello |
| CI | fmt + clippy (3 OS) + test (3 OS) + audit + deny + e2e + bench + release (3 OS) |
| 真实部署 | ROG-X ↔ Gray-Cloud 双端生产同步 (自建 relay 替代 Tailscale) |
| 当前最新 Release | [v3.0.4](https://github.com/juice094/syncthing-rust/releases/tag/v3.0.4) |

---

## 二、仍未完成 / 明确冻结的项

| 项 | 类型 | 当前状态 | 目标处置 |
|:---|:---|:---|:---|
| 72h 耐久测试 | 准入线 | 基础设施就绪，待实际跑测 | **v3.1.0 阻塞项** |
| 高延迟/不稳定网络大文件传输优化 | 已知限制 | 自动重连 + keepalive 已就位，深层优化未做 | **v3.1.0~v3.2.0 渐进优化** |
| Symlink 同步 | 功能缺口 | 静默跳过 | **v3.2.0 候选**（需先完成安全设计） |
| Web GUI | 冻结 | 无计划 | **保持冻结**，TUI/托盘/REST 为主界面 |
| QUIC / MagicSocket | 冻结 | 仅 TCP + Relay v1 | **保持冻结**，未来仅评估不做承诺 |

---

## 三、优先级矩阵

采用 **风险 × 工作量 × 用户价值** 三维评估：

| 任务 | 风险 | 工作量 | 用户价值 | 优先级 | 目标版本 |
|:---|:---|:---|:---|:---|:---|
| **72h stress test** | 🔴 高（稳定性未知） | 大 | 🔴 高（生产可信度） | **P0** | v3.1.0 阻塞 |
| **高延迟网络传输优化** | 🟡 中（场景特定） | 中 | 🟡 中（校园网/VPS 用户） | **P1** | v3.1.0~v3.2.0 |
| **Symlink 同步** | 🟡 中（安全/平台差异） | 中 | 🟡 中（dotfiles/开发工作流） | **P2** | v3.2.0 |
| **监控/可观测性增强** | 🟢 低 | ✅ 已交付 | 🟡 中（运维友好） | **✅ v3.0.4** | Prometheus + Grafana + 告警 |
| **Web GUI** | — | 大 | 🟡 中 | **冻结** | 不计划 |
| **QUIC / MagicSocket** | — | 大 | 🟡 中 | **冻结** | 不计划 |

---

## 四、分阶段执行计划

### Phase 1：v3.1.0 — 生产耐久验证（P0 阻塞项）

**目标**：证明 syncthing-rust 可以在真实双节点场景下稳定运行 72 小时，无内存泄漏、无数据丢失、无索引污染。

**准入标准**：
1. 连续 72h 双向同步，churn 脚本持续增删改文件。
2. 结束时双端 `files_changed=0`、`scans=0`、0 条 BEP error code 3。
3. RSS 增长 < 10%（排除内存泄漏）。
4. 日志无 panic、无 unwrap 触发、无死锁。

**关键任务**：
- [ ] 使用 `cmd/syncthing/src/bin/stress_test.rs` + `cmd/syncthing/src/bin/monitor.rs` 在 ROG-X ↔ Gray-Cloud 上跑满 72h。
- [ ] 跑测期间启用 `scripts/72h_monitor.sh` 采集 RSS、连接状态、扫描计数。
- [ ] 跑测结束后执行 `scripts/check-sync-consistency.ps1` 校验。
- [ ] 若失败，按 `docs/KNOWN_ISSUES.md` §14/§15 流程修复并重新跑测。

**ADR-1**：72h  stress test 不通过，v3.1.0 不发版。

---

### Phase 2：v3.1.x — 高延迟/不稳定网络优化（P1）

**目标**：降低校园网、跨国 VPS、Tailscale 高延迟链路下大文件传输的失败率。

**已知根因**：
- 块级响应超时对高 RTT 链路过短。
- Relay 健康检查在 dial 前串行执行，防火墙环境下阻塞 ~2min。
- 大文件批量 block request 对防火墙 session 不友好。

**优化方向**（按收益/风险排序）：
1. **可配置超时矩阵**：`block_request_timeout`、`relay_health_timeout` 从常量改为配置项，默认保守。
2. **请求分片 + 指数退避**：单文件块请求分批发送，失败块单独重试，避免一次性重传整个文件。
3. **Relay dial 并行化**：健康检查与直接 TCP dial 同时启动，先通先用。
4. **动态块大小**：高延迟链路下切换更大块尺寸，减少 RTT 往返次数（实验性，需 A/B 验证）。

**验收标准**：
- 在 ROG-X ↔ Gray-Cloud Tailscale 链路（RTT > 200ms）下，100MB 文件 5 次传输全部成功。
- 校园网防火墙环境 relay  dial 时间从 ~2min 降至 <30s。

---

### Phase 3：v3.2.0 — Symlink 同步（P2）

**目标**：在默认安全的前提下，支持同步符号链接关系。

**设计约束**：
- 默认行为：**跳过**（与当前一致），通过配置 `sync_symlinks: true` 开启。
- 仅同步**相对路径**软链；绝对路径软链必须转换或跳过。
- 必须检测并拒绝循环链接（`a -> b -> c -> a`）。
- Windows 平台创建软链需检测权限/开发者模式，无权限时降级为跳过并记录 warn。
- 路径逃逸检查：链接目标必须解析在同步目录内。

**任务拆分**：
- [ ] `syncthing-fs`：抽象 `Symlink` 类型，封装创建/读取/校验逻辑。
- [ ] `syncthing-sync`：Scanner 识别软链，生成 `FileInfo` 标志位；Puller 根据配置重建软链。
- [ ] `bep-protocol`：扩展 `FileInfo`（或复用现有字段）标记 symlink，确保与 Go Syncthing  wire 兼容。
- [ ] 安全测试：循环链接、绝对路径、`..` 逃逸、Windows 权限缺失。
- [ ] E2E 测试：双节点 dotfiles 仓库同步场景。

**ADR-2**：Symlink 同步默认关闭；开启后仅支持相对路径，且对端必须具备创建权限。

---

### Phase 4：v3.2.x — 监控与可观测性增强（P2）

**目标**：让生产部署更容易被观察和诊断。

**内容**：
- [ ] 完善 `/metrics`（Prometheus）端点：暴露同步文件夹计数、块下载速率、连接重连次数、watcher 事件队列长度。
- [ ] 结构化日志：为关键事件（扫描完成、连接建立、版本归档、冲突发生）添加 machine-readable 字段。
- [ ] 健康检查脚本：`scripts/check-health.ps1` 增加 REST API 探测与指标阈值告警。

---

## 五、明确冻结项（不再纳入版本计划）

### 5.1 Web GUI

- **冻结理由**：
  - 与项目“零运行时依赖、单二进制”目标冲突；Web GUI 需要打包前端资源或引入嵌入式浏览器。
  - TUI + Windows 托盘 + REST API 已覆盖主要交互场景。
  - 单人维护成本过高。
- **状态**：仅在收到高价值外部贡献或商业模式变化时重新评估。
- **参考**：[`AGENTS.md`](../../AGENTS.md) §10.5 冻结声明。

### 5.2 QUIC / MagicSocket

- **冻结理由**：
  - 当前 TCP + Relay v1 已满足生产穿透需求；QUIC 带来的收益（0-RTT、更好的 NAT 穿透）对现有用户不是阻塞项。
  - 引入 QUIC 会显著增加 TLS/网络层复杂度，与“核心路径代码不外包”约束冲突。
- **状态**：作为长期研究项保留，不做版本承诺。
- **参考**：[`AGENTS.md`](../../AGENTS.md) §10.5 冻结声明。

---

## 六、关键决策记录（ADR）

| ID | 决策 | 理由 |
|:---|:---|:---|
| ADR-1 | 72h stress test 是 v3.1.0 发版硬门槛 | Production 声明需要长期运行证据，而非仅单元测试 |
| ADR-2 | Symlink 默认关闭，仅支持相对路径 | 防止绝对路径跨设备失效、避免路径逃逸安全风险 |
| ADR-3 | 高延迟优化优先调超时与并行化，而非协议重写 | 单人维护约束下，小步快跑收益最高 |
| ADR-4 | Web GUI / QUIC 维持冻结 | 目标用户场景已覆盖，成本收益比不成立 |

---

## 七、度量指标

| 指标 | 当前值 | v3.1.0 目标 | v3.2.0 目标 |
|:---|---:|---:|---:|
| 测试通过数 | 433 | ≥ 440 | ≥ 450 |
| CI jobs 全绿 | 20/20 | 保持 | 保持 |
| 72h stress test | 未跑 | ✅ 通过 | 保持通过 |
| Symlink 同步 | 不支持 | 不支持 | ✅ 支持（可配置关闭） |
| `/metrics` 覆盖 | 基础 | 基础 | 完整 |

---

## 八、维护节奏

- **每周**：查看 CI / Dependabot / Issues，处理阻塞性 bug。
- **每月**：审查一次本计划，更新进度与已知限制。
- **每版本**：发布前更新 `CHANGELOG.md`、`AGENTS.md`、README 版本徽章、Release 说明。

---

*最后更新：2026-07-20 — v3.0.4 发布后更新（测试基线 414/6、CI 20 jobs、归档链接修正）*
