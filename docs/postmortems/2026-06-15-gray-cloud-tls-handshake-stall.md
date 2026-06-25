---
type: Postmortem
title: 2026-06-15 Gray-Cloud TLS Handshake Stall
description: Gray-Cloud 节点在 72h 压测准备阶段出现 daemon 停滞、TLS 握手超时的故障复盘与时间线。
resource: ./2026-06-15-gray-cloud-tls-handshake-stall.md
tags: [postmortem, tls, handshake, gray-cloud, stall, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# 2026-06-15 Gray-Cloud TLS 握手停滞事件

## 摘要

在 72h 压力测试准备阶段，Gray-Cloud（`IYGOGGD-...-RB3UDA4`）连续两次出现 **daemon 进程仍在，但停止写日志、停止接受新 TLS 连接** 的故障。ROG-X 侧表现为对 Gray-Cloud 的直接地址与所有 relay 候选的 TLS 握手全部超时。最终通过 `rm -rf db` 清空 Gray-Cloud 本地数据库并重启 daemon 后恢复。

初步结论：**不是网络波动，是程序设计/状态相关的问题**——daemon 在某种状态下进入“活着但不再处理连接/日志”的停滞态，TCP 端口仍开放，但 TLS 握手无法完成。

---

## 时间线（UTC）

| 时间 | 事件 |
|------|------|
| 11:51:38 | ROG-X → Gray-Cloud 首次 TLS 握手成功，BEP 连接建立 |
| 11:54:57 | ROG-X 重启 daemon（为启用 stress-test-72h） |
| 11:56:26 | ROG-X 开始自动重连 Gray-Cloud |
| 11:56:37 ~ 12:11:42 | **连续 TLS handshake timeout**（direct + relay） |
| 12:11:42 | 突然握手成功，连接恢复（原因未明，可能 Gray-Cloud 侧某些阻塞自然释放） |
| 12:47 左右 | Gray-Cloud 停止写日志（据 Gray-Cloud 侧报告） |
| 13:01:10 | ROG-X 报告 `Device disconnected ... - stale connection` |
| 13:01:22 ~ 14:31:48 | **再次连续 TLS handshake timeout**，重试计数涨到 14+ |
| 14:31:48 | Gray-Cloud `rm -rf db` 并重启后，ROG-X 收到 incoming Server TLS handshake，连接恢复 |
| 14:36:11 | 出现一次 `stale connection` 断连，但 4 秒后自动重连成功并稳定 |

---

## 症状与证据

### ROG-X 侧：TLS 握手超时，但 TCP 端口开放

```text
Parallel dialing IYGOGGD-... with 1 direct + 9 relay candidates
Failed to dial IYGOGGD-...: Timeout: TLS handshake timeout
Scheduling reconnect to IYGOGGD-... in ~300s (retry_count=14)
```

- `Test-NetConnection 100.69.11.71:22001` 返回 `True`。
- ping Tailscale 地址正常（~340 ms RTT）。
- 直接 TCP 与多个 relay 候选均出现同样超时，说明问题在 **TLS 层或更上层的 accept 处理**，而非单一网络路径。

### Gray-Cloud 侧：进程存活但日志静默

- 两次故障模式相同：daemon 进程还在，但日志在 20:47（本地时间，UTC 12:47）后停止写入。
- 清空 `db/` 并重启后立刻恢复正常。
- 这表明 daemon 内部状态（很可能是 sled DB 或连接/设备注册表）进入了一种让 accept 循环或 TLS 握手处理挂起的状态。

### 伴随的块请求异常

ROG-X 日志中出现对以下文件的 `remote error code 2` 或 `InvalidFile`：

- `stress-test-72h/100mb.bin`
- `stress-test-72h/churn_00000025_1048576.bin`（DB 重建期间）
- 若干 `note-XXXX.md`
- `claw-workspace/memory/2026-06-07.md` 等

这说明 **Gray-Cloud 的索引里包含它实际无法提供的文件**。DB 清空后这些文件从索引消失，但 `100mb.bin` 仍留在 stress-test-72h 中，可能是之前测试遗留。

---

## 初步诊断

### 排除网络波动

1. **TCP 通，TLS 不通**：网络层没有丢包或端口不可达；握手阶段超时。
2. **所有 relay 都超时**：如果仅是 direct 路径问题，relay 应能工作。
3. **重启 Gray-Cloud daemon（并清 DB）立刻恢复**：同一网络环境下，重启后同一地址/端口可正常握手。

### 指向程序设计/状态问题

1. **accept 循环或 TLS 握手任务可能被阻塞**
   - 可能原因： sled DB 查询、设备 ID 验证、或某个同步任务持有锁，导致新连接的 TLS acceptor 无法及时响应。
   - 10 秒握手超时（`TLS_HANDSHAKE_TIMEOUT = 10s`）被持续突破。

2. **DB 状态污染导致设备列表丢失**
   - Gray-Cloud 清空 DB 后恢复，说明异常退出可能把设备/索引状态写坏，新 daemon 启动后找不到 ROG-X 设备，握手处理可能因此挂起或静默失败。
   - 但 TLS 握手本应在 BEP ClusterConfig 之前完成，若仅是设备列表缺失，通常应表现为握手成功后关闭连接，而非握手超时。因此更可能是 **状态加载路径阻塞了 accept 线程/任务**。

3. **stale connection 检测与重连机制暴露的问题**
   - ROG-X 在 13:01:10 和 14:36:11 两次报告 `stale connection`，说明 Gray-Cloud 已不再响应心跳，但 ROG-X 侧连接仍保持。
   - 这符合“daemon 活着但事件循环停止”的假设。

---

## 建议后续行动

1. **在 Gray-Cloud 侧添加进程健康检查**
   - 监控 daemon 日志最后更新时间；超过 2 分钟无日志即视为“假死”，自动重启。
   - 这比单纯看进程是否存在更可靠。

2. **增加 TLS 握手阶段的日志**
   - 在 `syncthing_net::tls` 的 `accept_tls` / `connect_tls` 中加入更细粒度日志（例如开始握手、收到 ClientHello、证书验证结果）。
   - 当前日志只能看到“开始握手”和“超时”，无法判断是客户端没发、服务端没回，还是服务端在处理中被卡住。

3. **检查 sled / DB 加载路径的阻塞可能性**
   - `syncthing-db` 在启动或设备查询时是否有同步 I/O 或锁竞争？
   - 是否在 TLS 握手前需要访问 DB？如果是，应将设备认证延迟到握手完成后，避免 accept 循环被 DB 阻塞。

4. **清理 stress-test-72h 中的异常文件**
   - 删除 Gray-Cloud 端无法提供但仍在索引中的 `100mb.bin` 等遗留文件，避免 `remote error code 2` 和 `InvalidFile` 干扰测试。

5. **复现与压测**
   - 在测试环境人为 kill -9 daemon 多次，观察重启后是否出现同样“活着但不握手”状态。
   - 若可复现，用 `tokio-console` 或 `tracing` 查看 async 任务是否有被长时间阻塞的 accept/DB 任务。

---

## 追加发现：ROG-X daemon 9.6 小时死锁/忙等

次日（2026-06-16）早晨检查时发现更严重的同类问题出现在 **ROG-X 本机**：

| 时间（UTC） | 事件 |
|---|---|
| 15:46:20 | ROG-X daemon 日志停止；churn 脚本同时卡住 |
| 15:46 ~ 01:20 | **约 9.6 小时无 daemon 日志**，但 `syncthing.exe` 进程持续占用 ~100% CPU |
| 01:20:39 | daemon 终于输出：`Heartbeat timeout for IYGOGGD-... (idle 34618.6s)`，清理 69 个 pending response，标记 stale connection |
| 01:20 ~ 01:22 | daemon CPU 回落，自动重连 Gray-Cloud，BEP 恢复正常 |

### 影响

- **churn 脚本被阻塞**：PowerShell 进程仍在，但自 15:46 后未再创建/修改/删除任何文件。
- **monitor 继续采样**：monitor 只读取进程 RSS 和文件计数，未受影响；记录到 CPU 100% 的持续高占用。
- **BEP 连接假死**：对端 Gray-Cloud 在这 9.6 小时内也没有检测到正常心跳，说明 ROG-X 的事件循环/会话层卡住了，而不是单纯的对端问题。

### 诊断意义

这证明问题**不限于 Gray-Cloud 端**，也不是 DB 污染独有。`syncthing` daemon 本身在特定状态下可能进入 **单线程/事件循环死锁或忙等待**，表现为：

- 进程存在、CPU 高、日志停止；
- 对端心跳超时；
- 本地文件 I/O（churn 的文件创建）被间接阻塞；
- 经过极长时间后，内部的 heartbeat/stall 检测器才触发清理并恢复。

这强烈指向 **程序设计缺陷**：BEP 会话或同步组件中存在某种锁、无界循环或 async 任务饥饿，导致事件循环无法推进。

---

## 结论

本次异常 **不是网络波动**，而是 **syncthing-rust daemon 在两端都曾出现“进程存活但事件循环/BEP 会话停滞”的状态**。Gray-Cloud 侧与 DB 污染有关，ROG-X 侧则表现为长达 9.6 小时的死锁/忙等。两者共同特征是：

- TCP 端口可能仍开放；
- TLS 握手或 BEP 心跳无法完成；
- 日志停止写入；
- 重启/超时清理后可恢复。

需要在 BEP 会话状态机、心跳/stall 检测、DB 加载路径以及事件循环阻塞点上做根本性的排查与加固，否则 72h 长测会反复被此类假死打断。
