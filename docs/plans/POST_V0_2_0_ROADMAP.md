# Post-v0.2.0-beta 开发路线图

> **制定原则**：风险驱动开发（Risk-Driven Development）+ 技术债务优先 + 单人维护约束
> **制定日期**：2026-04-26
> **维护者**：juice094（单人维护，Windows 环境，零运行时依赖优先）

---

## 一、当前状态快照

| 维度 | 状态 |
|------|------|
| 功能完成度 | Phase 1~5 核心集成完成（BEP / Network / Sync / TUI / Discovery→CM） |
| 测试 | 279 passed, 1 ignored, 0 failed |
| 静态检查 | 0 clippy warnings |
| 安全审计 | **4 indirect warnings**（lru unsound, paste/instant/fxhash unmaintained） |
| 外部阻塞 | P0 格雷端 BEP 互通验证（等待格雷确认网络状态） |

---

## 二、目的与能力判定

### 2.1 核心目的

让 `syncthing-rust` 从 **"Beta 可用"** 演进为 **"可信的替代实现"**。可信的标准不是功能多，而是：

1. **无已知安全债务**（cargo audit clean）
2. **长期运行不泄漏、不崩溃**（72h stress test 通过）
3. **与 Go Syncthing 的互操作经过实际网络验证**（P0 解阻塞后）
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
| **cargo audit 清理**（4 warnings） | 🔴 高（安全/信任） | 中 | **P0** | 技术债务随时间指数增长；lru unsound 可能导致 UB |
| **72h stress test** | 🔴 高（稳定性未知） | 大 | **P1** | 无法宣称"生产可用"直到通过长时间验证 |
| **L3-APIW sub-gaps** | 🟡 中（功能缺口） | 小 | **P2** | YAGNI 的反面：已有 API 接口但行为不完整，属于"未完成的工作" |
| **.stignore pattern matching** | 🟡 中（兼容性缺口） | 中 | **P2** | Go Syncthing 用户迁移时的预期功能；无此功能会导致意外同步 |
| **L1-PM2 PCP/NAT-PMP** | 🟢 低（UPnP 已覆盖多数场景） | 中 | **P3** | 增量优化，非阻塞；家用路由器 UPnP 普及率 >80% |
| **Active Push scheduling** | 🟢 低（Pull 已工作） | 大 | **P4** | 性能优化，非功能阻塞；Go Syncthing 也以 Pull 为主 |
| **QUIC transport** | 🟢 低（TCP+Relay 已覆盖） | 大 | **P5** | 未来探索；需等 TCP 路径完全稳定后再评估 |

---

## 四、分阶段执行计划

### Phase A：安全债务清理（预计 1~2 天）

**目标**：`cargo audit` → 0 warnings

#### A1. lru (RUSTSEC-2026-0002, unsound)
- **路径**：ratatui 0.29.0 → lru 0.12.5
- **行动**：
  1. 合并 Dependabot 的 lru 更新 PR（若已存在）
  2. 若 Dependabot PR 未覆盖，尝试升级 `ratatui` 0.29 → 0.30
  3. 升级后验证 TUI 编译与基础交互（上下键、Tab 切换）
- **验收**：`cargo audit` 不再报 lru

#### A2. paste (RUSTSEC-2024-0436, unmaintained)
- **路径**：ratatui 0.29.0 + netlink-packet-core → paste 1.0.15
- **行动**：
  1. ratatui 升级至 0.30 可能自动解决
  2. 若 netlink-packet-core 仍报 paste，检查 `netdev` 是否有新版本（当前 0.41.0）
  3. 若 netdev 升级困难，评估是否接受此警告（unmaintained ≠ 有漏洞）
- **验收**：`cargo audit` 不再报 paste，或明确记录为"可接受债务"

#### A3. instant (RUSTSEC-2024-0384, unmaintained)
- **路径**：
  - parking_lot 0.11.2 → instant（via sled 0.34.7）
  - notify-types 1.0.1 → instant（via notify 7.0.0）
- **行动**：
  1. notify 7.0.0 → 检查 notify 8.x 是否移除 instant 依赖；若升级无 breaking change，执行
  2. sled 0.34.7 → sled 是核心存储，升级/替换成本高；若 sled 1.0 已发布且 API 兼容，评估迁移；否则接受警告并记录
- **验收**：notify 路径清理；sled 路径记录决策

#### A4. fxhash (RUSTSEC-2025-0057, unmaintained)
- **路径**：sled 0.34.7 → fxhash
- **行动**：与 A3 合并决策；sled 的升级/替换一次解决两个警告
- **备选**：若 sled 无法升级，考虑用 `rocksdb` 或 `redb` 替换，但需评估 API 兼容性与构建复杂度

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
- 调研 Go Syncthing 的 ignore 语法（gitignore 风格 + Syncthing 扩展如 `#include`）
- 评估复用现有 Rust crate：`ignore`（ripgrep 同款）或手写 matcher
- 集成到 `Scanner`：扫描前过滤路径，不计算被忽略文件的 block hash

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

### ADR-001：为什么不先做 .stignore 再做 audit？
- **理由**：lru unsound 是潜在内存安全漏洞，可能在 stress test 中触发；.stignore 是功能缺失，不会导致崩溃。
- **原则**：安全债务 > 功能债务（CWE-119 类比）

### ADR-002：为什么不直接替换 sled？
- **理由**：sled 0.34.7 当前工作稳定，替换为 rocksdb/redb 需要重构 `syncthing-db` 的存储抽象，工作量 >2 天。
- **原则**：单人维护项目，"能工作且无明显 bug"的依赖不应为了警告而替换，除非警告升级为 vulnerability。

### ADR-003：72h stress test 为什么不是自动化 CI？
- **理由**：72h 测试无法在 GitHub Actions 免费 runner 上完成（有时间限制）。
- **原则**：本地长期运行 + 日志分析是更务实的方案；未来可考虑自托管 runner。

---

## 七、即时行动清单（Next 24h）

- [ ] 合并 Dependabot lru PR（GitHub 网页操作）
- [ ] 本地 `git pull` + `cargo audit` 验证 lru 是否消除
- [ ] 尝试 `ratatui` 0.29 → 0.30 升级，TUI smoke test
- [ ] 尝试 `notify` 7.0 → 8.x 升级，编译验证
- [ ] 若上述升级引入 breaking change，记录并回滚，不硬啃
