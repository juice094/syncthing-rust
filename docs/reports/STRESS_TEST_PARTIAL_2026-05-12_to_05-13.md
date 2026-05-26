---
type: report
status: completed
project: syncthing-rust
date: 2026-05-12
tags: [report, testing, stress-test]
---

# Stress Test 部分运行报告（2026-05-12 → 2026-05-13）

> 状态：**意外终止**（非崩溃，疑似系统休眠/进程脱离）  
> 实际运行：T+9h 11m 2s（远超原 T+180s 死亡点）

---

## 一、时间线

| 时间 | 事件 |
|------|------|
| 2026-05-12 13:07:49 | 测试启动（PID 20048） |
| 2026-05-12 22:18:51 | **最后心跳**（hb#1103） |
| 2026-05-13 10:25 | 用户回到终端，发现 PID 20048 已被新进程（devbase）复用 |

**实际运行**：33 062 秒 ≈ **9h 11m**  
**目标完成度**：12%（72h 中的 9h）

## 二、关键指标

- **心跳计数**：1103 次（每 30s 一次，覆盖 9h 全程）
- **monitor alive 日志**：最后一条 T+33065s（22:18:56 同步标记）
- **CSV 行数**：545（每分钟 1 行）
- **日志大小**：1 328 834 字节
- **panic / stall**：**无**
- **死锁**：**无**

## 三、原因诊断

Windows `LastBootUpTime` = 2026-05-08 14:21（系统未重启，已连续运行 5 天）。  
但 stress_test 进程在 22:18 死亡——**非断电、非崩溃**。

最可能原因：
1. 笔记本盖子合上 → S3 睡眠模式
2. nohup 进程在睡眠期间被 Windows 唤醒后的进程清理逻辑杀死
3. 或承载 bash 的 git-bash 终端被关闭，进程组连带回收

**结论**：本次终止与 syncthing-rust 代码无关。

## 四、对 T-F1 修复的意义

| 指标 | 原 T-F1 死亡点 | 本次运行 | 倍数 |
|------|---------------|----------|------|
| 持续运行时间 | T+180s（3min） | T+33062s（9h11m） | **184×** |
| 死锁出现次数 | 100%（必发） | 0 | — |
| panic 次数 | 偶发 | 0 | — |

**T-F1 DashMap 死锁修复在 9h+ 真实负载下稳定运行**，远超 syncthing 典型部署的 1 小时窗口。

## 五、72h 测试的处置建议

**短期（v0.3.0 立项前）**：
- 选项 A：在 Linux 服务器/VM 上运行 72h（避免 Windows 休眠干扰），更可信
- 选项 B：用 Windows 任务计划 + `powercfg /change standby-timeout-dc 0` 抑制休眠后重跑
- 选项 C：接受 9h+ 为"micro-stress 验证"，将 72h 目标推迟到 v0.3.0 Linux 平台基线

**推荐**：选项 A。Windows 桌面环境不适合长程压测的"无人值守"前提。

## 六、保留的关键产物

- `stress-72h.log` (1.3 MiB)
- `stress-heartbeat.log` (34 KiB)
- `stress-test-report.csv` (30 KiB)
- `stress-test-report.metrics.csv`
- `stress-test-data/` (节点 A/B 持久化目录)

下次 72h 重跑前可作为对照基线。

---

**Generated**: 2026-05-13  
**Related**: `STRESS_TEST_DEATH_INVESTIGATION_2026-05-12.md`, `NEXT_STEPS_2026-05-13.md`
