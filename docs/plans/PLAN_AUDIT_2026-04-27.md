# 计划审计报告 · syncthing-rust

> **审计日期**: 2026-04-27
> **审计范围**: `docs/plans/` 全部 6 份计划 + `AGENTS.md` + 代码实际状态交叉验证
> **方法**: 计划声明 ↔ `git log` ↔ 文件系统实际存在性 ↔ `cargo audit` 输出 三方对账

---

## 一、逐份计划时效性判定

### 1. `MVP_RECOVERY_PLAN.md` — ⚠️ 过时，建议归档

| 维度 | 评估 |
|------|------|
| 制定时间 | 无明确日期，从内容推断为 4 月初 |
| 基线 commit | 无 |
| 完成度 | Phase 1~3 已真实完成；Phase 4（完整消息循环）标注"未开始"，但后续计划已覆盖 |
| 问题 | 文档中存在两个 `## 4. 结论`（第 149 行和第 189 行），是仓促拼接产物 |
| 建议 | **归档到 `docs/archive/`**。其历史价值仅限于记录早期断裂点诊断，不再指导当前开发。 |

### 2. `PHASE3_PLAN.md` — ⚠️ 部分过时，需勘误

| 维度 | 评估 |
|------|------|
| 基线 commit | `90776e7`（2026-04-17），存在但仅为文档/traits 重构，非功能里程碑 |
| 执行记录 | `2026-04-20` 声称"核心功能全部完成 ✅" |
| **关键疑点** | 3.3 节声称"Push E2E + Pull E2E 完成（**Cloud Go 节点**双向验证）"，但当前已知格雷侧为 **pre-fix Rust 构建**，非 Go。该声明的真实性存疑。 |
| 未完成项 | 3.4 节 `72h stress test infra` 所有 checkbox 为 `[ ]`，确实未启动 |
| 建议 | **保留但顶部添加勘误横幅**，修正"Go 节点"为"格雷侧远程节点（当时误认为是 Go，实际为旧版 Rust）"；3.4 节保持不变。 |

### 3. `PHASE4_PLAN.md` — ❌ 严重过时，建议归档并重写

| 维度 | 评估 |
|------|------|
| 基线 commit | `e8882ca`（2026-04-20），存在但仅为 README/计划文档更新，非功能 commit |
| Week 排期 | Week 1~3 排期已过期（当前为 4 月 27 日） |
| **虚假声明 #1** | "连接循环（双向拨号竞争）✅ 已完成" — `git log` 中**无任何 commit**与此对应。竞争解决逻辑实际在 `manager.rs` 拆分后的 `registry.rs` 中存在，但从未被标记为独立完成的里程碑。 |
| **虚假声明 #2** | "配置持久化 ✅ 已完成" — 部分完成，但 `daemon_runner.rs` 中仍有硬编码 `test_mode` 残留 |
| 未完成项 | 4.3 压测、4.4 生产打包均未启动 |
| 建议 | **归档到 `docs/archive/`**。TUI 功能补齐和压测计划应由新路线图接管，避免基于过期时间表决策。 |

### 4. `POST_V0_2_0_ROADMAP.md` — ✅ 最新，但 P0 基于错误假设，需修正

| 维度 | 评估 |
|------|------|
| 制定日期 | 2026-04-26，是 6 份计划中最新的 |
| **P0 错误假设** | 声称"4 warnings（lru unsound + 3 unmaintained）"。实际 `cargo audit` 输出：**lru 0.16.4 已无任何警告**，仅剩 3 个 `unmaintained`。数量从 4 降到 3，且性质从"unsound"全变为"unmaintained"。 |
| **P0 不可行性** | 所有直接依赖（`ratatui`、`netdev`、`sled`）均已是最新稳定版。`paste`/`instant`/`fxhash` 的警告位于上游传递依赖，本项目无法通过 semver 升级消除。 |
| 重复问题 | 与 `improvement-plan.md` 高度重复，却互不引用，像是独立起草 |
| 建议 | **原地更新**：下调 cargo audit 为 P3/P4；补充"已自动解决"项（lru、ratatui、notify）；重写 Phase A 为"监控上游"而非"主动清理"。 |

### 5. `WAVE3_PLAN.md` — ⚠️ 大部分已完成，建议归档

| 维度 | 评估 |
|------|------|
| 制定时间 | 无日期，从任务代号推断为 4 月上旬 |
| 完成度 | NET-REBIND（`netmon.rs`）、NET-DIALER（`dialer.rs`）、SYNC-SUPERVISOR（`supervisor.rs`）均已在代码库中实现 |
| 建议 | **归档到 `docs/archive/`**。保留历史价值，但不再指导开发。 |

### 6. `improvement-plan.md` — ⚠️ 过于理想化，建议降级后归档

| 维度 | 评估 |
|------|------|
| 制定时间 | 无明确日期，内容引用 2026-04-20 |
| **Exit Criteria 不切实际** | "生产代码零 unwrap" — 当前代码库中存在大量合理的 `unwrap()`（如 `mpsc::unbounded_channel()` 的 sender 初始化、`DashMap::new()` 等），强行归零会引入过度工程化。<br>"REST API 与 Go Syncthing GUI 完全兼容" — 当前甚至未与 Go 节点完成验证，"完全兼容"是远期目标而非 v0.2.0 退出条件。<br>"cargo audit 零漏洞" — 如上所述，3 个 unmaintained 无法从本项目消除。 |
| 建议 | **归档到 `docs/archive/`**。其工作流分类（A~F）有参考价值，但优先级和 Exit Criteria 需要重写。 |

---

## 二、代码声明 vs 实际状态对账

| 声明来源 | 声明内容 | 实际状态 | 偏差等级 |
|----------|----------|----------|----------|
| `AGENTS.md` | "`daemon_runner.rs` 当前 858 行" | 实际 **465 行** | 🔴 严重（夸大 85%） |
| `PHASE4_PLAN.md` | "连接循环 ✅ 已完成" | `git log` 无对应 commit | 🔴 严重 |
| `PHASE3_PLAN.md` | "Cloud Go 节点双向验证 ✅" | 格雷侧实际为旧版 Rust | 🔴 严重 |
| `POST_V0_2_0_ROADMAP.md` | "4 warnings（含 lru unsound）" | 实际 3 个 unmaintained，lru 已 clean | 🟡 中等 |
| `PHASE4_PLAN.md` / `POST_V0_2_0_ROADMAP.md` | "scripts/stress_test.sh + analyze_stress.py" | 仅有 `.ps1` 脚本，无 `.sh`/`.py` | 🟡 中等 |
| `PHASE4_PLAN.md` | "tests/stress_72h.rs" | **不存在**；仅 `bin/stress_test.rs` 存在 | 🟡 中等 |
| `improvement-plan.md` | "iroh 死代码已清理" | 已确认完成 | ✅ 一致 |
| `AGENTS.md` | "`.stignore` ✅ 已完成" | `syncthing-fs/src/ignore.rs` 存在但标 `DO NOT USE IN PRODUCTION`；`syncthing-sync/src/ignore.rs` 为简化版 | 🟡 中等 |

---

## 三、项目阶段性定位重定义

### 当前不应声称的定位

| 定位 | 为什么不对 |
|------|-----------|
| "生产就绪" | 72h stress test 未执行；无长期运行数据；unwrap 未清理 |
| "Go Syncthing 完全兼容" | 未与 Go 节点完成完整文件同步验证 |
| "v0.2.0 Exit Criteria 已满足" | 6 条 Exit Criteria 中至少 4 条未满足 |

### 建议的准确表述

> **syncthing-rust 当前处于"功能完整的 Beta"阶段**：
> - BEP 协议核心消息（Hello/ClusterConfig/Index/Request/Response）编解码与握手完整
> - TCP+TLS+Relay 多路径传输已实现
> - REST API 读端完整，写端部分完成
> - TUI 基础状态观测可用
> - **未验证项**：72h 长期稳定性、跨版本 Rust 互通、Go 互操作、大文件/海量文件场景

### 阶段性目标（重新校准）

| 阶段 | 目标 | 验收标准 |
|------|------|----------|
| **v0.2.0-beta.1**（当前） | 功能完整，可编译，测试通过 | 294 passed, 0 failed, clippy clean |
| **v0.2.0-beta.2** | 长期稳定性基线建立 | 72h stress test 通过（本地或格雷远程） |
| **v0.2.0-rc.1** | 跨版本互通验证 | 新版 Rust ↔ 旧版 Rust（格雷侧）完成双向文件同步 |
| **v0.2.0** | 生产候选 | Go Syncthing 互操作验证通过；REST API 写端闭环；cargo audit 接受债务并文档化 |

---

## 四、后续目标实现路径（修正后）

### 真实 P0：72h Stress Test 执行

- 已有 `cmd/syncthing/src/bin/stress_test.rs`（290 行），无需重新开发
- **下一步**：本地执行短周期验证（2~4 小时），确认无 panic / 无内存泄漏趋势
- 短周期通过后，移交格雷远程执行 72h 完整测试

### 真实 P1：跨版本 Rust 互通验证

- 目标：新版 `main` ↔ 格雷侧 pre-fix 构建
- 路径：格雷侧升级到 `main` 最新 commit 后，先验证同版本互通；再回退格雷侧到旧版，测试兼容性
- **当前阻塞**：格雷侧网络连通性（os error 10061）

### 真实 P2：REST API 写端闭环

- `POST /rest/db/override` / `revert` — 从 501 stub 实现为真实调用
- `POST /rest/system/pause` / `resume` — `device` body 参数生效
- `POST /rest/db/scan` — `sub` 子路径参数支持

### 降级为 P3：cargo audit 债务接受

- 创建 `.cargo/audit.toml`，显式接受 3 个 unmaintained 警告
- 在文档中记录决策理由（ADR）
- 不再视为主动开发任务

### 冻结项（明确不投入）

- TUI 新功能（设备详情页、文件夹详情页、向导、带宽图表）
- 生产打包（systemd/MSI）
- Go Syncthing 互操作（等 Rust 内部稳定后再评估）
- BEP 扩展 Verify 消息族、跨实例共识

---

## 五、文件清理行动清单

| 文件 | 行动 | 负责人 |
|------|------|--------|
| `MVP_RECOVERY_PLAN.md` | 移动到 `docs/archive/` | Agent |
| `PHASE4_PLAN.md` | 移动到 `docs/archive/` | Agent |
| `WAVE3_PLAN.md` | 移动到 `docs/archive/` | Agent |
| `improvement-plan.md` | 移动到 `docs/archive/` | Agent |
| `PHASE3_PLAN.md` | 保留，顶部添加勘误横幅 | Agent |
| `POST_V0_2_0_ROADMAP.md` | 原地更新 P0/P1/P2 | Agent |
| `AGENTS.md` | 修正 `daemon_runner.rs` 行数；更新阶段性定位 | Agent |
| `docs/plans/PLAN_AUDIT_2026-04-27.md` | 本文件，已创建 | Agent |

---

*审计完成。建议人类确认阶段性定位重定义后，再启动下一阶段开发。*
