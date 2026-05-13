# Stress Test 9h11m 完整分析报告

> 测试期：2026-05-12 13:07:49 → 2026-05-12 22:18:51（CST，本地时间）  
> 测试 PID：20048（自然结束于系统休眠）  
> 数据源：`stress-72h.log` (1.3 MiB), `stress-test-report.csv` (553 行), `stress-heartbeat.log` (1103 行)  
> 目标：72h，实际：9h11m2s（12% 完成度）  
> **承接**：[`STRESS_TEST_PARTIAL_2026-05-12_to_05-13.md`](./STRESS_TEST_PARTIAL_2026-05-12_to_05-13.md)（速记版）

---

## 0. 执行摘要

| 维度 | 结论 |
|------|------|
| **T-F1 死锁修复** | ✅ 完全验证。原 T+180s 100% 冻结的死锁路径在 9h11m 内**完全无复现**。 |
| **连接层稳定性** | ✅ 758 次连接周期，0 panic，0 deadlock，0 task crash。 |
| **BEP 握手** | ✅ 1516 次 Hello 交换全部成功（758 入站 + 758 出站）。 |
| **端到端同步** | ⚠️ **未验证**。TestNode 测试架构层级到 BEP Hello 即止，未驱动 BepSession → ClusterConfig → Index → Block 流水线。 |
| **心跳超时复发** | ⚠️ 每 90s 一次（HEARTBEAT_INTERVAL 默认值），共 743 次。这是 **预期行为**（无业务流量），但表明 stress_test 未模拟真实负载。 |
| **内存稳定性** | ⚠️ 不可观测。`rss_mb` 在 CSV 中恒为 0（监控代码进程名匹配 bug，T-F1 周期内 patch 已 push 但本次 binary 未启用此版本）。 |
| **休眠对长跑的影响** | ❌ 致命。Windows 桌面下 nohup 子进程在 sleep/hibernate 后被系统回收。72h 计划必须迁移到 Linux/VM。 |

---

## 1. 时间线

```
T+0s        13:07:49  stress_test 启动，PID 20048
T+5s        13:07:55  Node A ↔ Node B 首次连接握手成功
T+~90s      13:09:19  首个 Heartbeat timeout（90s idle 触发）
T+~90s      13:09:21  race resolution 自动重连成功
…           …         之后每 ~90s 重复一次连接周期（共 758 次）
T+33065s    22:18:56  最后一条 monitor alive 日志
T+33062s    22:18:51  最后一次心跳（hb#1103）
─────────────────────────────────────────────
T+~33100s   ~22:19    系统进入睡眠/休眠，进程被回收
2026-05-13 10:25 用户回到终端，发现进程已不存在；PID 20048 已被另一进程复用
```

---

## 2. 关键指标（与原 T-F1 故障对比）

| 指标 | 原 T-F1 故障 | 本次运行 | 改善倍数 |
|------|--------------|----------|----------|
| 持续运行时间 | T+180s（百分百冻结） | T+33062s（自然结束） | **184× ↑** |
| 死锁次数 | 必发 | 0 | — |
| Panic 次数 | 偶发 | 0 | — |
| Task crash | 偶发 | 0 | — |
| 心跳超时次数 | 不适用（早死） | 743（每 90s 1 次） | 行为符合 spec |

T-F1 deadlock 路径（`ConnectionManager::register_connection` 跨 `.await` 持 DashMap 写锁）在压测中**完全无复现**。代码层面通过 `RegisterAction` enum 模式重写后，决策与 await 完全分离，从源头杜绝了该死锁。

---

## 3. 日志事件统计（颜色码已清洗）

```
1516 INFO bep_protocol::handshake     (Hello sent + Hello received × 758 对)
 767 INFO syncthing_net::manager::registry   (Connection registered / Closing)
 762 INFO syncthing_net::tls          (TLS handshake completed × 2 端 × 757)
 760 INFO syncthing_net::transport::bep_adapter
 758 INFO syncthing_net::manager      (Scheduling reconnect)
 758 INFO syncthing_net::handshaker   (BEP hello exchange complete)
 590 INFO stress_test                 (monitor alive + heartbeat 输出)
 379 INFO syncthing_net::dialer       (Parallel dialing)
   8 INFO syncthing_sync::service     (启停 + cookie)
   8 INFO syncthing_net::connection   (connection close)
   2 INFO syncthing_net::transport::tcp
─────────────────────────────────
 743 WARN syncthing_net::connection   (Heartbeat timeout)
   0 ERROR …
```

**0 个 ClusterConfig 或 Index 相关日志**。这印证了 TestNode 没有完整启动 BEP session pipeline。

---

## 4. 监控 CSV 解读

```
Header:   timestamp,elapsed_secs,connected_a_b,connected_b_a,
          folder_state_a,folder_state_b,files_a,files_b,errors,rss_mb
Rows:     553（首行 5s，末行 33065s，间隔 60s）
```

| 列 | 全程取值 | 解释 |
|----|---------|------|
| `connected_a_b` | `true` × 553 | A → B 连接寿命覆盖所有采样点（race resolution 保证瞬时无空窗） |
| `connected_b_a` | `true` × 553 | B → A 同上 |
| `folder_state_a` | `present` × 553 | A 节点的 stress-folder 一直存在 |
| `folder_state_b` | `present` × 553 | B 同 |
| `files_a` | 1 → 6 | A 节点的 sliding-window 文件数（创建/修改/删除）|
| `files_b` | **0 × 553** | ⚠️ B 始终是 0 — 同步未发生 |
| `errors` | 0 × 553 | 注入器没有 IO 错误 |
| `rss_mb` | 0 × 553 | 监控代码 bug（已 fix 但旧 binary 未生效） |

### 4.1 时间戳格式 bug
首行：`20585T05:07:55Z`。这是 chrono format string 错误（缺 `-`），实际应为 `2026-05-12T05:07:55Z`。
建议在后续 binary 修正后再启用本字段。

---

## 5. 文件残留分析

测试结束时 `stress-test-data/` 目录状态：
```
node-a/sync/
  file_0106.dat
  file_0107.dat
  file_0108.dat
  file_0109.dat
  file_0110.dat
  file_0111.dat
node-b/sync/      ← 空
```

inject_task 创建到 file_0111（共 ~111 次循环），但 sliding window 仅保留最近 6 个。  
B 节点 sync 目录为空 = **同步从未发生**。

---

## 6. 根因分析：为何 ClusterConfig 没发出？

`TestNode::new_with_dir` 启动的组件：
- ✅ `SyncService`
- ✅ `ConnectionManager`
- ❌ **缺少** `manager.on_connected` 回调中创建 `BepSession::with_events` 的桥接代码

完整的 daemon 流（`cmd/syncthing/src/tui/daemon_runner.rs::start_daemon`）才会：
1. 注册 `on_connected` 回调
2. 在回调内 `BepSession::new()` 并 `.run()`
3. BepSession 内部发送 ClusterConfig、Index、IndexUpdate
4. 收到对端 Index 后由 puller 拉取 Block

TestNode 测试架构跳过了步骤 1-4，只验证到「TLS+Hello+register」即止。

### 影响范围
- 本次压测**不能证明**：BEP session 长跑稳定性、Index 同步、Block 拉取、puller 重试、folder model 等
- 本次压测**能证明**：连接层、握手层、TLS、race resolution、heartbeat、reconnect 调度

---

## 7. 改进建议

### v0.3.0 短期（高优先级）
1. **TestNode 增强**：在 `harness.rs` 中加入 BepSession 启动逻辑，模拟 `start_daemon` 的连接回调。预期工作量：3-5h。
2. **stress_test 重设计**：增加端到端校验（B 侧文件计数 / hash 比对）。预期工作量：2h。
3. **rss_mb 修复**：T-F1 周期已 patch（进程名 `stress_test`），下次 build 即可生效。
4. **CSV 时间戳格式**：修正 chrono format string `%Y-%m-%dT%H:%M:%SZ`（少 `-`）。

### v0.3.0 中期
5. **Linux 平台 72h 验证**：迁移到 WSL2 / VM / 远程 VPS。Windows 桌面不适合无人值守长跑。
6. **HEARTBEAT_INTERVAL 配置化**：当前硬编码 90s。对真实负载场景应根据流量自适应。
7. **Idle keepalive PING**：在无业务流量时仍发送 PING 维持连接，避免 90s 周期性断开重连。

### v0.4.0 长期
8. **跨版本互通压测**：syncthing-rust ↔ Go syncthing 2.x 长程互通 72h。
9. **故障注入压测**：模拟网络抖动、丢包、TLS cert 过期、磁盘满。

---

## 8. T-F1 验证的最终评定

| 维度 | 验证状态 |
|------|----------|
| 死锁修复（核心目标） | ✅ **完全验证**（184× 时长，0 复现） |
| 连接层稳定性 | ✅ **强证据**（758 次完整周期） |
| 长跑（72h 目标） | ⏳ **需要 Linux 重测** |
| 端到端同步 | ⏳ **需要 TestNode 增强后重测** |

T-F1 deadlock 修复在工程意义上**已经验证**。继续追求 72h Windows 桌面运行没有 ROI；应转向：
1. 改进 TestNode 以覆盖完整 BEP 流水线
2. 在 Linux 服务器上做正式 72h 跑测

---

## 9. 关联文档

- 死锁 RCA：[`STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md`](./STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md)
- 部分运行速记：[`STRESS_TEST_PARTIAL_2026-05-12_to_05-13.md`](./STRESS_TEST_PARTIAL_2026-05-12_to_05-13.md)
- 性能 baseline：[`BASELINE_2026-05-12.md`](./BASELINE_2026-05-12.md)
- 持锁审计：[`LOCK_AWAIT_AUDIT_2026-05-12.md`](./LOCK_AWAIT_AUDIT_2026-05-12.md)
- 会话归档：[`SESSION_SUMMARY_2026-05-12.md`](./SESSION_SUMMARY_2026-05-12.md)
- 当前活动计划：[`../plans/NEXT_STEPS_2026-05-13.md`](../plans/NEXT_STEPS_2026-05-13.md)

---

**Generated**: 2026-05-13  
**Author**: juice094（与 AI 协作分析）  
**Status**: T2.1 完成 ✅
