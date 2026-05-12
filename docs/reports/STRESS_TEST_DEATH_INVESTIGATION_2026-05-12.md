# 72h Stress Test 进程死亡调查报告

**调查时间**: 2026-05-12 12:00-12:50 UTC+8
**调查者**: T-F1 增强诊断
**结论**: 部分定位，需更深入调查

---

## 关键发现

### 1. 现象总览
- **症状**：stress_test.exe 在 T+~180–210s 后**静默死亡**
- **不是 Rust panic**：未生成 `stress-crash.log`（panic hook 完整覆盖）
- **stderr 为空**：无 abort/segfault 输出
- **Windows Application Error 无事件**：Get-WinEvent 查询 ID=1000/1001/1002 均空
- **System log 无关闭/电源事件**：ID=41/1074/6008 均空
- **Windows Defender 无 Block/Detection 事件**：但有大量 cloud lookup (Event 2010)

### 2. 死亡时间模式（高度一致）
| 启动方式 | 启动时间 | 最后日志 | 心跳停止 | 进程死亡 |
|---------|---------|---------|---------|---------|
| PowerShell Start-Process | 12:36:20 | 12:36:25 (T+5s) | 12:36:20 (T+0s) | T+~180s |
| bash nohup & | 12:40:34 | 12:43:39 (T+185s) | 12:44:04 (T+210s) | T+~210–240s |
| 计划任务自动启动 | 12:49:26 | 进行中 | 进行中 | 推测 T+~210s |

### 3. PowerShell Start-Process 问题
`Start-Process -RedirectStandardOutput -RedirectStandardError -WindowStyle Hidden` 在 PowerShell 父进程退出后**破坏子进程的 stdout/stderr 文件句柄**，导致：
- tracing_subscriber 写入失败
- tokio runtime 似乎冻结（所有任务无法写入）
- 进程残存但 CPU 接近 0
- 约 T+180s 后被某种机制终止

**这解释了 PowerShell 启动的 stress_test 仅有 T+5s 一条监控日志和 T+0s 一条心跳的原因**。

### 4. bash nohup 启动相对正常但仍死亡
使用 `bash> ... > log 2> err &` 启动：
- 心跳每 30s 写入 → 共 8 条 (T+0, +30, +60, ..., +210)
- 监控任务每 60s 写入 → 共 4 条 (T+5, +65, +125, +185)
- 但 T+~210s 后心跳停止，进程死亡

## 待解谜团

### 为什么 bash 启动也会在 T+~210s 死亡？

**已排除**：
- Rust panic（无 crash log）
- OOM（内存稳定 12MB）
- 显式 process::exit（stress_test 内无此调用）
- Defender 阻止（无相关 Event）
- 系统休眠/重启（uptime 持续）

**待验证**：
- bash 会话终止时 SIGHUP 是否真的被 nohup 忽略？
- tokio runtime 是否在大量 BEP race resolution 后内部断言失败？
- 是否有 thread/handle 限制？
- 文件句柄是否在某个时间点变得无效？

## 验证措施（已实施）

### T-F1 增强项
1. **Panic hook 全局注册** ✅
   - 任何 Rust panic 必生成 `stress-crash.log`
   - 包含完整 backtrace（`std::backtrace::Backtrace::force_capture()`）
2. **主线程心跳** ✅（本次新增）
   - 每 30s 写入 `stress-heartbeat.log`
   - 独立于 tracing 子系统
   - 用于区分"runtime 冻结" vs "外部终止"
3. **监控任务频率** ✅（本次调整）
   - 由 600s 调整为 60s（长运行模式）
   - 早期死亡可见性大幅提升
4. **stderr 捕获** ✅
   - 守护脚本添加 `-RedirectStandardError`
   - RUST_BACKTRACE=full

## 待执行下一步

1. **改用文件 logger（不依赖 stdout）** — 引入 tracing-appender
2. **添加 Windows ETW 监听** — 监控进程终止事件
3. **使用 Process Explorer 拍快照** — 在 T+~200s 时检查线程栈
4. **缩小 BEP 范围测试** — 不启用 fault/inject，仅观察连接是否仍会死
5. **添加 Defender 排除** — 临时关闭实时保护或排除 stress_test.exe

## 当前可观察现实

- TestNode 模式（不跑完整守护进程）下连接 race resolution 极频繁
  - 6 次连接注册 vs 8 次 TLS 握手 → 2 次因 race 关闭
  - 这种"快速建立-竞争-关闭"循环可能耗尽某种 Windows 资源
- BEP 连接生命周期管理需要更深入审计
