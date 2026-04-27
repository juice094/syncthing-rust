# Post-v0.2.0-beta 开发路线图

> 📋 **计划索引**：[`docs/plans/INDEX.md`](./INDEX.md) — 查看所有计划文件关系与跳转速查
> 📋 **审计报告**：[`docs/plans/PLAN_AUDIT_2026-04-27.md`](./PLAN_AUDIT_2026-04-27.md) — 计划时效性审计与定位重定义
>
> **制定原则**：风险驱动开发（Risk-Driven Development）+ 技术债务优先 + 单人维护约束
> **制定日期**：2026-04-26（审计修正 2026-04-27）
> **维护者**：juice094（单人维护，Windows 环境，零运行时依赖优先）

---

## 一、当前状态快照

| 维度 | 状态 |
|------|------|
| 功能完成度 | Phase 1~5 核心集成完成（BEP / Network / Sync / TUI / Discovery→CM） |
| 测试 | 294 passed, 3 ignored, 0 failed |
| 静态检查 | 0 clippy warnings |
| 安全审计 | **3 unmaintained warnings**（paste/instant/fxhash），均为上游传递依赖，无漏洞。`lru` 已自动消除（0.16.4）。 |
| 外部阻塞 | 格雷端 BEP 互通验证（等待格雷确认网络状态） — 不阻塞开发主路径 |

---

## 二、目的与能力判定

### 2.1 核心目的

让 `syncthing-rust` 从 **"功能完整的 Beta"** 演进为 **"可信的替代实现"**。可信的标准不是功能多，而是：

1. **已知安全债务已评估并接受**（cargo audit 3 个 unmaintained 为上游传递依赖，实际风险可忽略；已用 `.cargo/audit.toml` 显式记录）
2. **长期运行不泄漏、不崩溃**（72h stress test 通过）
3. **与 Go Syncthing 的互操作经过实际网络验证**（远期目标，等 Rust 内部稳定后再评估）
4. **REST API 功能闭环**（读写完整，配置管理无需重启）

### 2.2 能力约束（Hard Constraints）

- **单人维护**：任何引入的新依赖/模块必须能在 <2 天内理解、修改、回滚
- **Windows 主力**：所有测试必须在 Windows 原生通过（无 WSL 假设）
- **零 Docker**：不引入容器化测试基础设施
- **Rust 核心不可外包**：关键路径代码（ dialer / TLS / BEP 握手）不依赖子 Agent 生成
- **不追新**：不为了升级而升级，升级依赖的唯一理由是解决安全警告或阻塞性 bug

---

## 三、优先级矩阵

采用 **风险 × 工作量** 二维评估：

| 任务 | 风险等级 | 工作量 | 优先级 | 理论依据 |
|------|----------|--------|--------|----------|
| **72h stress test 执行** | 🔴 高（稳定性未知） | 大 | **P0** | 无法宣称"Beta 可信"直到通过长时间验证；已有 `bin/stress_test.rs`，只需执行 |
| **跨版本 Rust 互通验证** | 🔴 高（当前阻塞） | 中 | **P0** | 新版 `main` ↔ 格雷侧 pre-fix Rust 的实际连通性；解阻塞后冻结新功能修 bug |
| **L3-APIW sub-gaps** | 🟡 中（功能缺口） | 小 | **P1** | YAGNI 的反面：已有 API 接口但行为不完整，属于"未完成的工作" |
| **cargo audit 债务接受** | 🟢 低（unmaintained ≠ vulnerable，无 actionable 路径） | 小 | **P3** | 所有直接依赖均已最新稳定版；3 个警告位于 sled/netlink-packet-core 上游，无法从本项目消除 |
| **.stignore pattern matching** | 🟡 中（兼容性缺口） | 中 | **P2** | `syncthing-sync/src/ignore.rs` 简化版已可用；`syncthing-fs` 完整版标为 unverified，按需激活 |
| **L1-PM2 PCP/NAT-PMP** | 🟢 低（UPnP 已覆盖多数场景） | 中 | **P3** | 增量优化，非阻塞；家用路由器 UPnP 普及率 >80% |
| **Active Push scheduling** | 🟢 低（Pull 已工作） | 大 | **P4** | 性能优化，非功能阻塞；Go Syncthing 也以 Pull 为主 |
| **QUIC transport** | 🟢 低（TCP+Relay 已覆盖） | 大 | **P5** | 未来探索；需等 TCP 路径完全稳定后再评估 |

---

## 四、分阶段执行计划

### Phase A：安全债务评估与接受（预计 0.5 天）

**目标**：明确记录已知安全债务，将 `cargo audit` 从"开发阻塞"降级为"监控项"。

#### A1. lru — 已自动消除 ✅
- **现状**：`Cargo.lock` 中 `lru 0.16.4`，`cargo audit` 无警告。
- **结论**：无需任何行动。

#### A2. paste — 上游阻塞，无 semver 升级路径
- **路径**：`netdev 0.42.0`（最新稳定）→ `netlink-packet-core 0.8.1`（最新稳定）→ `paste 1.0.15`
- **结论**：`netdev` 和 `netlink-packet-core` 均已最新。`paste` 是编译期宏 crate，运行时暴露为零。**接受为债务。**

#### A3. instant + A4. fxhash — sled 内部依赖，替换风险过高
- **路径**：`sled 0.34.7`（最新稳定，1.0 alpha 已停滞 3 年）→ `parking_lot 0.11.2` → `instant`；`sled` → `fxhash`
- **结论**：`sled` 是核心存储层，升级至 1.0 alpha 或替换为 rocksdb/redb 均需 >2 天重构 + 数据兼容性验证。单人维护约束下不可接受。**接受为债务。**

#### A5. 创建 `.cargo/audit.toml`
- **行动**：显式忽略 3 个 unmaintained 警告，附决策理由。
- **验收**：`cargo audit --no-fetch` 输出 `0 warnings`（忽略项不报错）。

---

### Phase B：72h Stress Test（预计 3~5 天执行）

**目标**：验证长期运行下的内存、连接、同步稳定性。

**现状**：`cmd/syncthing/src/bin/stress_test.rs`（290 行）已存在，无需重新开发。

#### B0. 本地短周期预验证（2~4 小时）
- **行动**：本地执行 `cargo run --bin stress_test --release`，观察 2~4 小时
- **监控**：是否 panic、RSS 是否持续增长、连接是否稳定
- **验收**：短周期无异常，再移交格雷远程执行 72h

#### B1. 格雷远程 72h 执行
- **行动**：将 release 二进制 + 执行指南移交格雷，在远程 Linux 服务器运行
- **监控指标**：内存 RSS、连接存活、文件同步延迟、错误日志数
- **验收标准**：
  - 72h 内无 panic、RSS 增长 < 50%
  - 所有注入文件最终同步一致（SHA-256 比对）
  - 重连成功率 > 95%

### Phase B：72h Stress Test（预计 3~5 天设计与执行）

**目标**：验证长期运行下的内存、连接、同步稳定性

#### B1. 测试方案设计
- **场景**：2 个 `TestNode` 实例，共享一个文件夹，持续运行 72 小时
- **注入负载**：
  - 每 5 分钟在 Node A 创建/修改/删除小文件（1KB ~ 10MB）
  - 每 30 分钟触发一次网络断开（模拟 WiFi 切换）→ 验证 reconnect
  - 每 2 小时触发一次 config reload → 验证 hot-reload 不泄漏
- **监控指标**：
  - 内存 RSS（每小时采样）
  - TCP 连接数 / goroutine（tokio task）数
  - 文件同步延迟（从写入到对端收到 IndexUpdate）
  - 错误日志数（ERROR/WARN 级别）

#### B2. 测试基础设施
- 复用 `test_harness.rs` 的 `TestNode`
- 新增 `cmd/syncthing/src/bin/stress_test.rs` 或 `tests/stress_72h.rs`
- 输出 CSV 日志，便于事后分析

#### B3. 验收标准
- 72h 内无 panic、无内存泄漏（RSS 增长 < 50%）
- 所有注入文件最终同步一致（SHA-256 比对）
- 重连成功率 > 95%

### Phase C：API & 兼容性补全（预计 2~3 天）

#### C1. L3-APIW sub-gaps
- `POST /rest/system/pause` / `resume` — `device` body 参数实际生效（当前仅处理 `folder`）
- `POST /rest/db/scan` — `sub` 参数支持子路径扫描
- `POST /rest/db/override` / `revert` — 从 501 stub 改为实际实现（调用 sync_service）

#### C2. .stignore pattern matching
- **现状**：`syncthing-sync/src/ignore.rs`（241 行，简化版）已支持基本 glob/negation/anchoring；`syncthing-fs/src/ignore.rs`（830 行，完整版）标为 `DO NOT USE IN PRODUCTION`。
- **行动**：评估简化版是否覆盖 90% 场景；若不足，审计并清理完整版后启用。

### Phase D：增量网络优化（P3，按需启动）

#### D1. L1-PM2 PCP/NAT-PMP
- 在 `PortMapper` 中实现 PCP 和 NAT-PMP 协议（参考 RFC 6886, RFC 6887）
- 优先级低于 UPnP，作为 fallback 路径

---

## 五、与外部阻塞（P0 格雷验证）的协同

格雷验证是外部阻塞项，**不应占用开发带宽等待**。正确策略是：

1. **并行推进**：在格雷确认期间，执行 Phase A/B/C
2. **准备就绪**：Phase A/B 完成后，项目处于"随时可验证"状态
3. **验证即冻结**：一旦格雷确认可用，暂停新功能开发，全力解决互通中发现的问题

---

## 六、决策记录（ADR）

### ADR-001（已作废，2026-04-27 审计后重定义）：为什么不将 cargo audit 作为 P0？
- **旧理由**：lru unsound 是潜在内存安全漏洞。
- **新事实**：lru 0.16.4 已无任何警告；剩余 3 个均为 `unmaintained`，无 CVE，无运行时暴露，且全部位于上游传递依赖，无法从本项目消除。
- **新结论**：`cargo audit` 清理不应占用开发带宽。已创建 `.cargo/audit.toml` 显式接受，转为监控项。

### ADR-002：为什么不直接替换 sled？
- **理由**：sled 0.34.7 当前工作稳定，替换为 rocksdb/redb 需要重构 `syncthing-db` 的存储抽象，工作量 >2 天。
- **原则**：单人维护项目，"能工作且无明显 bug"的依赖不应为了警告而替换，除非警告升级为 vulnerability。

### ADR-003：72h stress test 为什么不是自动化 CI？
- **理由**：72h 测试无法在 GitHub Actions 免费 runner 上完成（有时间限制）。
- **原则**：本地长期运行 + 日志分析是更务实的方案；未来可考虑自托管 runner。

---

## 七、即时行动清单（Next 24h）

- [x] 审计全部 6 份计划书，识别过时/虚假声明
- [x] 归档 4 份过时计划（MVP_RECOVERY / PHASE4 / WAVE3 / improvement-plan）
- [x] 修正 PHASE3 勘误（Go 节点 → 格雷侧旧版 Rust）
- [ ] 创建 `.cargo/audit.toml`，接受 3 个 unmaintained 警告
- [ ] 本地执行 `bin/stress_test` 短周期预验证（2~4h）
- [ ] 更新 AGENTS.md 修正 `daemon_runner.rs` 行数等错误信息
