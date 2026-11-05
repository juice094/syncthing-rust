---
type: ADR
title: ADR-20261105-03: 设备 ID 格式（Base32 + Luhn-32）
description: Syncthing 设备 ID 的编码、校验与显示格式，及版本向量计数器 ID 语义
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-03: 设备 ID 格式（Base32 + Luhn-32）

> English summary: Device IDs are 32-byte SHA-256 hashes encoded as unpadded
> Base32 (RFC 4648) with Luhn-32 check characters, plus legacy 64-char hex
> parsing for compatibility; the first 8 bytes double as the version-vector
> counter ID (Go ShortID semantics).

## 格式定义

设备 ID 是设备证书的 SHA-256 哈希（32 字节），字符串形式为：

1. **Base32 编码**（RFC 4648 字符集，无填充）：32 字节 × 8 位 = 256 位，
   256 / 5 = 51.2，故编码为 52 字符；不足位以 `'A'`（Base32 的 0）右填充。
2. **Luhn-32 校验位**：52 个字符分成 4 组，每组 13 数据字符 + 1 校验位，
   得到 56 字符；校验算法与 Go Syncthing 的 `luhn32` 一致。
3. **显示分组**：56 字符按 8 组 × 7 字符以 `-` 连接
   （`XXXXXXX-XXXXXXX-...`）。
4. **旧格式兼容**：解析端同时接受 64 字符 hex（旧版编码）。

解析时先剥离所有非字母数字字符（分隔符与空白），再按长度（64 = hex，
56 = Base32+Luhn-32）分派。

## 版本向量计数器 ID

`DeviceId::counter_id()` 取前 8 字节的大端 u64，作为该设备在版本向量中的
计数器 ID（与 Go Syncthing 的 `ShortID` 语义一致）。

**约束**：每个设备在版本向量中必须使用唯一 ID，否则两端各自的修改会被
误判为线性历史而非并发，导致删除方"支配"并静默覆盖对端的离线修改。

## 理由

Base32 + Luhn-32 与 Go 实现逐字节互通，是人可读、可手抄、带检错能力的
格式；Luhn-32 能在输入阶段捕获绝大多数转录错误。计数器 ID 复用 ID 前缀
避免了额外的 ID 分配机制，同时要求全网唯一性以保证冲突检测正确性。
