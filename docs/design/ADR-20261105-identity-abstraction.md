---
type: ADR
title: ADR-20261105-02: Identity 抽象分层
description: Identity trait 与具体密码学方案解耦的设计论证
status: active
project: syncthing-rust
date: 2026-11-05
---

# ADR-20261105-02: Identity 抽象分层

> English summary: The `Identity` trait decouples device identification from
> concrete cryptography schemes so one device can hold several identities and
> each transport can pick the scheme that fits it best.

## 问题

syncthing-rust 要从"文件同步工具"演进为"多传输安全网络层"。若设备标识与
某一密码学方案（TLS）硬绑定，新增传输（QUIC、WireGuard、PSK）时标识层
必须重写。

## 设计原则

- **最小接口**：只定义"我是谁"，不涉及握手细节（握手属于传输层）。
- **零依赖**：不引入 tokio_rustls、wireguard 等具体库；`syncthing-core`
  保持对下游 crate 只读。
- **可扩展**：具体实现（如 `TlsIdentity`）位于上层 crate（如
  `syncthing-net`）。

## 决定

`Identity` 只负责"标识"，不负责"握手"。握手逻辑归属传输层：

- `TcpTransport` 使用 `TlsIdentity` 做 TLS 握手；
- `QuicTransport` 使用内置 TLS 1.3 或 `WireGuardIdentity`；
- `WebSocketTransport` 使用 `TlsIdentity` 做 WSS 握手。

## 理由

这种分层让同一设备可以同时拥有多种身份（如 TLS + WireGuard 双栈），
不同传输可以选择最适合的身份方案，而设备 ID（32 字节标识符）在所有
方案间保持不变，保证 `DeviceId` 作为全网统一标识的稳定性。
