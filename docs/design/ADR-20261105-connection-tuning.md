---
type: ADR
title: ADR-20261105-connection-tuning：心跳、尺寸上限与 keepalive 参数
description: BEP 连接的心跳间隔按路径类型调优、消息尺寸上限收紧、空闲超时检测与 TCP keepalive 参数集的选型理由
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-connection-tuning：心跳、尺寸上限与 keepalive 参数

> English summary: Tuning decisions for BepConnection: per-path-type heartbeat
> intervals (90 s direct default capped by path type), a 64 MiB
> MAX_BEP_MESSAGE_SIZE (tightened from 128 MiB), idle-timeout dead-connection
> detection (3 consecutive header-read timeouts), and TCP keepalive parameter
> sets (60s/10s/3 direct vs 30s/5s/2 through middleboxes). Rationale for each
> value lives here, not in code comments.

## 心跳间隔（按路径类型）

默认心跳 **90 s**（`HEARTBEAT_INTERVAL`），可由配置通过
`set_default_heartbeat_interval` 覆盖（`ConnectionManager::new` 注入）。
`BepConnection::new` 再按传输类型封顶：

| 路径类型 | 心跳间隔 |
|:---|:---|
| TCP 直连 | 基础值（默认 90 s） |
| Relay / Proxy / WebSocket / DERP 等中间路径 | `min(基础值, 30 s)` |
| Memory（测试） | `min(基础值, 10 s)` |

**理由**：中间节点（relay 服务器、HTTP 代理、WebSocket 网关、DERP）通常
在 60–90 s 内回收空闲连接，比 NAT/防火墙（5–15 分钟）激进得多；封顶 30 s
保证 keepalive 先于中间节点的回收窗口到达。直连路径保留宽松值以避免
不必要的流量。

判活：心跳任务在 `last_pong` 距今超过 `3 × heartbeat_interval` 时关闭
连接。**任何收到的消息都是存活证据**——接收循环每收一帧就刷新
`last_pong`（此前该字段初始化后从不更新，导致连接在固定
`3 × heartbeat_interval` 后被误杀）。

## 消息尺寸上限

`MAX_BEP_MESSAGE_SIZE` = **64 MiB**（2026-05-11 自 128 MiB 收紧，
TUNING_PLAN T-D4）。

**理由**：Syncthing 实际 Index 上限约 30 MiB（参考 Go 实现）；128 MiB 给
攻击者留下的放大空间过大。Header 上限保持 64 KiB。

## 空闲超时死连接检测

读取任务对 2 字节 header length 的读操作应用 `DEFAULT_MESSAGE_TIMEOUT`
（60 s）；连续超时 **3 次**判定连接已死并返回错误，避免在僵尸连接上
无限自旋。收到任何数据即重置计数。

## TCP keepalive 参数（Linux）

`apply_tcp_keepalive` 按传输类型选择参数集（非致命：平台不支持时记录
debug 并继续）：

| 路径 | time | interval | retries |
|:---|---:|---:|---:|
| 直连 TCP | 60 s | 10 s | 3 |
| Relay / Proxy / WebSocket | 30 s | 5 s | 2 |

**理由**：与心跳间隔的选型同构——中间节点更早回收空闲连接，因此需要
更积极的 keepalive 探针；直连路径用保守参数避免产生多余流量。Relay
路径尤其脆弱（被中间节点静默掐断后上层无从感知），建立会话后立即应用。

## 理由汇总

所有取值遵循同一原则：**参数由路径上最激进的空闲回收者决定**。取值与
理由集中记录在本 ADR，代码中只保留指针，避免注释与实现漂移。
