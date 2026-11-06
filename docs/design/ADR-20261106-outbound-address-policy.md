---
type: ADR
title: Outbound Address Policy
description: 出站地址校验的 SSRF 策略开关与默认安全值。
resource: ./ADR-20261106-outbound-address-policy.md
tags: [adr, network, security, ssrf]
status: accepted
project: syncthing-rust
timestamp: 2026-11-06T00:00:00Z
---

# ADR: Outbound Address Policy

> **Implementation status (2026-11-06)**: Implemented. `Options.allow_private_targets`
> and `Options.allow_loopback_targets` exist, `OutboundPolicy` is threaded through
> `ConnectionManagerConfig.outbound_policy`, `dialer` passes it to
> `validate_outbound_addr`, and the production `daemon_runner` builds the policy
> from `Config.options`. Test harness explicitly relaxes loopback.

## Context

tcp_transport.rs 的 validate_outbound_addr 拒绝 multicast、unspecified、link-local，但放行 127.0.0.1、192.168/16、10/8、172.16/12。对局域网发现是合理设计，但恶意 peer 可诱导本机连接内网服务，形成 SSRF。当前无配置项让用户收紧。

调用点只有 dialer.rs 一处，改动面可控。

## Decision

### 配置契约

加在 Config.options 里（与 ADR-20261106-bandwidth-throttle 同结构体，字段并列）：

    pub struct Options {
        // 现有字段保持不变
        /// 是否允许出站连接到私有网段（192.168/16、10/8、172.16/12）。
        /// 默认 true，兼容局域网发现。
        pub allow_private_targets: bool,
        /// 是否允许出站连接到 loopback（127.0.0.0/8、::1）。
        /// 默认 false，防止诱导本机访问本地服务。
        pub allow_loopback_targets: bool,
    }

默认值按默认安全原则：
- allow_private_targets = true（保持现状，否则局域网发现全挂）
- allow_loopback_targets = false（收紧，堵住最常见的 SSRF 路径）

### 校验函数签名

改为 validate_outbound_addr(addr: &SocketAddr, policy: &OutboundPolicy) -> Result<()>。OutboundPolicy 从 Config.options 构造，调用点 dialer.rs 传入从 state 取出的策略。

### 粒度选择

全局开关，不做按 peer 区分。理由：按 peer 区分需要在 Device 结构加字段、在握手时协商、在校验函数里查表，复杂度显著上升，而收益是某个恶意 peer 用私网地址、另一个正常 peer 用公网地址这种极窄场景。全局开关已能覆盖我不想让任何 peer 把我当跳板的主流诉求。

### 配置可见性

随 Config.options 序列化，GET /rest/config 与 PUT /rest/config 自动可见。修改后需重启 daemon 生效，与 max_connections 一致，不引入热更新。

## Consequences

- 正面：默认堵住 loopback SSRF；用户可按需放行私网。
- 负面：allow_loopback_targets 为 false 会破坏单机双实例调试场景，需在文档明确告知调试时临时开启。
- 风险：全局开关无法处理多 peer 混合信任等级。若未来确有需求，可加 Device.allow_private_targets 覆盖全局值，不改本 ADR 的核心决策。

## Alternatives Considered

- 默认 allow_loopback_targets = true：保持现状，但 SSRF 风险敞口大。
- 按 peer 区分：复杂度高，当前无明确需求。
- 只拒绝 loopback 不加配置：破坏调试场景，用户无法绕过。
