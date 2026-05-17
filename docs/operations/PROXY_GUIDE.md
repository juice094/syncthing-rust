# 出站代理（SOCKS5 / HTTP）使用指南

> 目的：通过本地代理（如 Watt Toolkit / clash / v2rayN / shadowsocks）转发  
> syncthing-rust 的出站连接，用于加速国际链路或合规要求。
>
> 适用：v0.2.4 及以上 syncthing-rust  
> 维护：2026-05-13

---

## 0. TL;DR

```bash
# 1. 启动你的本地代理（例如 Watt Toolkit 默认监听 127.0.0.1:26561）

# 2. 在启动 syncthing-rust 前设置环境变量
# Windows CMD:
set SOCKS5_PROXY=127.0.0.1:26561
syncthing.exe --listen 0.0.0.0:22001

# PowerShell:
$env:SOCKS5_PROXY = "127.0.0.1:26561"
.\syncthing.exe --listen 0.0.0.0:22001

# Linux/macOS:
export SOCKS5_PROXY=127.0.0.1:26561
./syncthing --listen 0.0.0.0:22000

# 3. 所有出站 TCP 连接（BEP peer 拨号 + reqwest discovery/relay 拉取）
#    会自动经代理转发。
```

---

## 1. 支持的环境变量

来源：[`crates/syncthing-net/src/transport/proxy.rs`](../../crates/syncthing-net/src/transport/proxy.rs)

| 变量名（大写或小写） | 协议 | 优先级 |
|-------|------|--------|
| `SOCKS5_PROXY` / `socks5_proxy` | SOCKS5 | 1（最高） |
| `ALL_PROXY` / `all_proxy` | SOCKS5 | 2 |
| `HTTP_PROXY` / `http_proxy` | HTTP CONNECT | 3 |

格式：`host:port` 或 `socks5://host:port` 或 `http://host:port`

> ⚠️ 当前不支持 SOCKS5 用户名/密码认证。如需，在 [`POST_V0_2_0_ROADMAP.md`](../plans/POST_V0_2_0_ROADMAP.md) P3 列入。

---

## 2. 各代理工具的典型端口

| 工具 | 默认端口 | 备注 |
|------|----------|------|
| **Watt Toolkit**（原 Steam++） | 26561 | 仅 Windows/macOS；以 GitHub/Steam 加速为主 |
| **Clash for Windows** | 7890 (HTTP) / 7891 (SOCKS5) | 跨平台 |
| **v2rayN** | 10808 (SOCKS5) / 10809 (HTTP) | Windows |
| **shadowsocks-windows** | 1080 | Windows |
| **Surge / Quantumult X** | 6152 (HTTP) / 6153 (SOCKS5) | macOS / iOS |
| **HTTP CONNECT 通用** | 因配置而异 | 注意区分 forward / reverse |

具体端口请查阅各工具的"本地代理监听"设置。

---

## 3. 适用场景

### ✅ 推荐使用
1. **跨境 syncthing relay 加速**：拉取 `relays.syncthing.net/endpoint` 走代理可以解决境内连接失败问题
2. **企业出站审计**：所有外发连接必须经堡垒机/审计代理
3. **强制走特定网络出口**：例如多 ISP 链路中指定某条出口

### ⚠️ 部分有效
4. **BEP peer 直连国外**：如果对端在境外公网，代理可以提速；但如对端也在境内，绕一圈反而变慢
5. **全球 discovery 服务可达性**：经代理可访问 `discovery.syncthing.net`

### ❌ 不要在此使用代理
6. **LAN 同步**：本地子网设备走代理纯属浪费
7. **Tailscale tailnet 内部连接**：100.x.y.z 是私有地址，代理只会失败
8. **入站连接**：环境变量只影响出站，不能让代理"代为接收"入站 BEP 拨号

---

## 4. 与 Tailscale 的优先级

二者**不冲突但通常二选一**：

| 选择 | 适用场景 |
|------|----------|
| **Tailscale** | 自管设备之间长期同步（家+笔记本+手机） |
| **代理** | 偶尔加速跨境 relay/discovery，或合规要求 |
| **同时启用** | 需要 Tailscale 拨号同时，HTTP 拉取走代理（生效但复杂） |

通常推荐：**家里设备用 Tailscale**（[`TAILSCALE_GUIDE.md`](./TAILSCALE_GUIDE.md)），**与外部 Go syncthing 互联时用代理**。

---

## 5. 关于 Watt Toolkit 的特殊说明

Watt Toolkit 的核心价值是 **hosts 文件注入加速 GitHub / Steam 等域名**。

### syncthing-rust 运行时连接的域名

| 目标域名 | 触发时机 | Watt 是否能加速 |
|----------|----------|----------------|
| `discovery.syncthing.net` | global discovery 心跳 | ❌（Watt hosts 表不含此域名） |
| `relays.syncthing.net` | 拉取 relay pool | ❌（同上） |
| `github.com` | **不连接** | — |
| `crates.io` | **不连接** | — |
| 对端 device IP | BEP 拨号 | ❌（纯 IP，无域名加速） |

**结论**：Watt Toolkit 对 syncthing-rust **运行时**几乎无收益。但若你将 syncthing-rust 通过 Watt Toolkit 的 SOCKS5 转发，可以利用 Watt 的网络优化（IPv6 / TCP BBR 等）间接加速。

### Watt Toolkit 在开发阶段更有用
- `git clone https://github.com/juice094/syncthing-rust` ：受益于 Watt 的 GitHub 加速
- `cargo install --git ...` ：同上
- `cargo fetch` 拉取 crates.io 依赖：Watt 不直接加速 crates.io，建议在 `~/.cargo/config.toml` 配 [中国镜像](https://rsproxy.cn/)

---

## 6. 故障排查

### 6.1 验证代理是否生效

启动前确认环境变量已设置：
```bash
echo $SOCKS5_PROXY     # bash
echo %SOCKS5_PROXY%    # cmd
$env:SOCKS5_PROXY      # powershell
```

启动后检查日志（含 `INFO syncthing_net::transport::proxy:`）：
```
INFO syncthing_net::transport::proxy: ProxyConfig loaded: socks5://127.0.0.1:26561
```

如果没看到这行，说明环境变量未读到。

### 6.2 代理拒绝连接

错误日志示例：
```
WARN syncthing_net::dialer: Parallel dialing FAILED: ProxyError("SOCKS5 handshake failed")
```

排查清单：
1. 代理软件是否已启动？`telnet 127.0.0.1 26561` 试试
2. 代理监听的是 IPv4 还是 IPv6？syncthing-rust 当前优先 IPv4
3. 代理是否支持 SOCKS5 的 CONNECT 方法（多数都支持）

### 6.3 性能反而下降

如果发现走代理后速度更慢：
- 移除环境变量回归直连
- 或在配置中关闭 `global_discovery_enabled` 减少经代理的元数据流量

---

## 7. 安全注意

- **明文 SOCKS5 / HTTP 代理**：代理服务器能看到全部加密前的元数据（连接源/目标 IP+端口），但**不能解密 BEP**（已经过 TLS）
- **避免在 untrusted 公共代理上**运行 syncthing-rust，会暴露你 tailnet 拓扑
- 推荐：本地代理（127.0.0.1）或自托管远程 SOCKS5 over WireGuard/Tailscale

---

## 8. 编程接口（高级）

如果你在嵌入式集成 syncthing-rust，可直接构造 `ProxyConfig`：

```rust
use syncthing_net::transport::proxy::{ProxyConfig, ProxyType};

let proxy = ProxyConfig {
    proxy_type: ProxyType::Socks5,
    address: "127.0.0.1:26561".parse().unwrap(),
};
// 注入到 TransportRegistry...
```

未来 v0.3.0 计划：
- ✅ SOCKS5 用户名/密码认证
- ✅ HTTP/HTTPS 鉴权
- ✅ 多代理链
- ⏳ 域名 split（境内直连 + 境外走代理）

---

## 9. 关联文档

- [`TAILSCALE_GUIDE.md`](./TAILSCALE_GUIDE.md) — Tailscale 零配置 NAT 穿透
- [`../../crates/syncthing-net/src/transport/proxy.rs`](../../crates/syncthing-net/src/transport/proxy.rs) — 代理实现源码
- [`../plans/POST_V0_2_0_ROADMAP.md`](../plans/POST_V0_2_0_ROADMAP.md) — 代理增强路线图

---

**Status**: 已验证（环境变量方案）  
**Last reviewed**: 2026-05-13
