---
type: guide
status: active
project: syncthing-rust
tags: [ops, deployment, tailscale, vpn]
---

# 与 Tailscale 协同部署指南

> 目的：用 Tailscale 提供的零配置 NAT 穿透 + WireGuard 加密 + Magic DNS，  
> 让 syncthing-rust 在任意网络环境（家庭路由器/4G 蜂窝/双层 NAT/公司防火墙）下"开箱即用"。
>
> 适用：v0.2.4 及以上 syncthing-rust  
> 维护：2026-05-13

---

## 0. TL;DR

```bash
# 在每台设备：
1. 安装 Tailscale 并登录（https://tailscale.com/download）
2. tailscale up                  # 加入你的 tailnet
3. tailscale ip                  # 获取本机 100.x.y.z 地址
4. 在 syncthing-rust 配置中，将对端 device 的地址填为 tcp://100.x.y.z:22000
5. 启动 syncthing-rust，它会通过 Tailscale 直接拨号

不需要修改任何源码，不需要绑定到 Tailscale 接口。
```

---

## 1. 为什么搭配 Tailscale？

### 1.1 syncthing-rust 当前的 NAT 穿透能力
| 机制 | 状态 | 局限 |
|------|------|------|
| **本地发现**（LAN UDP 广播） | ✅ 已实现 | 仅同子网 |
| **全球发现**（discovery.syncthing.net） | ✅ 已实现 | 依赖第三方服务，可能被 GFW 屏蔽 |
| **STUN 探测公网 IP** | ✅ 已实现 | 对称 NAT 无效 |
| **UPnP / NAT-PMP / PCP 端口映射** | ✅ 已实现 | 多数家用路由器关闭 UPnP |
| **Syncthing relay**（公共 relay pool） | ✅ 已实现 | 高延迟 + 限速 + 国内可达性差 |
| **DERP 协议**（自有实现） | ✅ 已实现（1480 行代码） | 需自建 DERP 服务器 |

**实际场景痛点**：当两台设备都在严格 NAT 后面（典型：手机 4G + 家里宽带），上述链路全部失败时只能走 relay，速度 < 1 MB/s。

### 1.2 Tailscale 解决什么
Tailscale 在内核态/用户态 WireGuard 之上实现了：
- 自动 NAT 穿透（包括对称 NAT、CGNAT）
- 失败时自动回退到全球 DERP 中继（Tailscale 维护数十节点）
- 端到端加密（WireGuard）
- 设备身份认证（基于 OIDC）
- 每设备分配稳定 IPv4 `100.64.0.0/10` 范围

将 syncthing-rust 部署在 Tailscale 之上，相当于把"NAT 穿透"这个责任完全外包给一个工业级实现，syncthing-rust 只需要在 `100.x.y.z` 这个稳定 LAN-like 网络上工作。

---

## 2. 部署步骤

### 2.1 在每台参与同步的设备上安装 Tailscale

| 平台 | 命令 |
|------|------|
| **Windows** | 下载安装包 https://tailscale.com/download/windows |
| **macOS** | App Store 或 `brew install tailscale` |
| **Linux** | `curl -fsSL https://tailscale.com/install.sh \| sh` |
| **iOS / Android** | App Store / Play Store |

登录到你的 tailnet（个人版免费，限 100 设备）：
```bash
sudo tailscale up   # 浏览器会打开 OAuth 登录
```

### 2.2 验证连通性

```bash
# 设备 A
$ tailscale ip
100.64.1.5
fd7a:115c:a1e0::1234

# 设备 B
$ tailscale ip
100.64.1.7
fd7a:115c:a1e0::5678

# 设备 A 测试到设备 B
$ ping -c 3 100.64.1.7
PING 100.64.1.7 56 data bytes
64 bytes from 100.64.1.7: time=23 ms
```

如果可以 ping 通，syncthing-rust 也能直接拨号。

### 2.3 配置 syncthing-rust

在每台设备的 `config.json` 中，将对端 device 的 `addresses` 字段填入 Tailscale IP：

```json
{
  "devices": [
    {
      "id": "AAAA-BBBB-...",   // 对端 device_id
      "name": "laptop",
      "addresses": [
        "tcp://100.64.1.7:22000",   // ← Tailscale IP
        "dynamic"                     // 仍保留 LAN 发现作 fallback
      ]
    }
  ]
}
```

或者通过 CLI 子命令：
```bash
syncthing-cli device add \
  --id AAAA-BBBB-... \
  --address tcp://100.64.1.7:22000
```

### 2.4 启动 syncthing-rust
```bash
syncthing --listen 0.0.0.0:22000
```

监听 `0.0.0.0` 确保接受来自 Tailscale 接口的入站连接。Tailscale 的 ACL 会自动限制只有 tailnet 内部的设备才能连接。

---

## 3. 进阶配置

### 3.1 关闭 syncthing 自带 NAT 穿透（节省资源）
当通过 Tailscale 时，syncthing 自带的 STUN/UPnP/NAT-PMP 是冗余的：

```json
{
  "options": {
    "stun_enabled": false,
    "upnp_enabled": false,
    "global_discovery_enabled": false,
    "relays_enabled": false
  }
}
```

只保留本地发现（LAN）和直接 IP 拨号（Tailscale）。

### 3.2 利用 Tailscale 的 Magic DNS

如果你启用了 Tailscale Magic DNS：
```bash
tailscale up --accept-dns
```

可以用主机名代替 IP：
```json
"addresses": ["tcp://laptop.tail-scale.ts.net:22000"]
```

### 3.3 限制只接受 Tailscale 入站连接

绑定到 Tailscale 网卡而非 `0.0.0.0`：
```bash
syncthing --listen 100.64.1.5:22000   # ← 本机 Tailscale IP
```

这样防火墙规则更简单（外部公网 + LAN 都连不进来），只能通过 tailnet。

---

## 4. 性能对照

下表是参考数据（具体取决于网络条件）：

| 链路 | 延迟 | 吞吐 | 备注 |
|------|------|------|------|
| LAN 直连 | <1 ms | ~1 Gbps | 最快 |
| Tailscale 直连（同城） | 5-20 ms | 100-500 Mbps | 通常 NAT 穿透成功 |
| Tailscale DERP 中继 | 30-100 ms | 20-50 Mbps | 双方都在严格 NAT 后时 |
| Syncthing relay pool（国内） | 80-300 ms | 1-10 Mbps | 不推荐 |

Tailscale DERP 在 99% 场景下显著优于 syncthing relay。

---

## 5. 故障排查

| 症状 | 可能原因 | 解决 |
|------|----------|------|
| ping 100.64.x.y 不通 | Tailscale 未启动 | `tailscale up` |
| ping 通但 syncthing 连不上 | 防火墙拦截 22000 | 放行入站 TCP/22000 |
| syncthing 连接频繁重连 | HEARTBEAT_INTERVAL 与 Tailscale 心跳冲突 | 现象但无害；v0.3.0 会修复 |
| 网速远低于预期 | 走了 DERP 中继 | `tailscale netcheck` 看是否 DERP；调整路由器降低 NAT 严格度 |

---

## 6. 不该用 Tailscale 的场景

- **完全自托管/合规要求**：使用 syncthing-rust 自带的 DERP 服务器（见 §7）
- **设备 > 100**：Tailscale 个人版上限，升级 Team 版（付费）或换 Headscale（开源自托管控制面）
- **无外网**：完全离线环境用 LAN + 本地发现

---

## 7. 备选方案：syncthing-rust 自带 DERP

仓库已实现完整的 DERP server/client（见 `crates/syncthing-net/src/derp/`，1480 行）。
v0.3.0 将通过 CLI 子命令把它暴露为：

```bash
# 在 VPS 上：
syncthing-cli derp-server --listen 0.0.0.0:3478

# 在客户端配置中：
"relays": ["derp://my-vps.example.com:3478"]
```

属于 v0.3.0 候选范围（参见 [`NEXT_STEPS_2026-05-13.md`](../plans/NEXT_STEPS_2026-05-13.md) 第 5 章）。

---

## 8. 关联文档

- [`PROXY_GUIDE.md`](./PROXY_GUIDE.md) — 通过 HTTP/SOCKS5 代理（如 Watt Toolkit / clash）转发出站连接
- [`../archive/plans/POST_V0_2_0_ROADMAP.md`](../archive/plans/POST_V0_2_0_ROADMAP.md) — NAT 穿透模块路线图（已归档）
- [`../reports/STRESS_TEST_REPORT_2026-05-13.md`](../reports/STRESS_TEST_REPORT_2026-05-13.md) — 9h+ 压测连接层稳定性证据

---

**Status**: 已验证（路径 A，零代码部署）  
**Last reviewed**: 2026-05-13
