# 每日收尾报告 — 2026-05-15

## 今日完成工作清单

### 一、Bug 修复（P0）
- [x] Bug-1: incoming 连接未设状态导致 `connected_devices()` 返回空
- [x] Bug-2: `with_extension` 产生 `..syncthing.tmp` 双点号
- [x] Bug-3: Scanner `Path::strip_prefix` Windows 平台差异
- [x] 本地验证：WSL2 ↔ Windows 双向 Block Transfer 成功
- [x] 测试报告更新：`docs/reports/DUAL_NODE_TEST_2026-05-15.md` §11
- [x] `KNOWN_ISSUES.md` §9 标记为已修复

### 二、配置 UX 重构
- [x] C-UX-1: CLI 初始化向导 (`syncthing init`)
- [x] C-UX-2: AddressType URL 序列化 (`tcp://host:port`)
- [x] C-UX-3: REST API 热添加设备 (`POST /rest/config/devices` + 后台 `connect_to`)
- [x] C-UX-4: 配置验证 + 快速失败 (`validate_config`)
- [x] C-UX-5: 单实例锁 (`syncthing.pid`)

### 三、基础设施
- [x] CI 强化：新增 `release-check`、`doc-check`、`e2e-test` job
- [x] 代码健康与解耦审计报告
- [x] 工程纪律规范文档

### 四、Git 状态
- 分支：`main`
- 领先 origin: **11 commits** (01429a2)
- 推送状态：✅ 全部已推送

---

## 未验证项（必须后续补齐）

| 项 | 当前状态 | 为什么必须补 |
|----|---------|-------------|
| 真实网络双节点测试 | ❌ 未做 | WSL2↔Windows 是虚拟网卡，非真实防火墙/NAT 环境 |
| Gray-Cloud 云服务器部署 | ❌ 未更新 | 对侧仍运行旧版本，未包含 Bug-1/2/3 修复 |
| C-UX-2 URL 格式 config 验证 | ⏳ 待验证 | 新序列化代码只在单元测试中验证，未在真实配置加载中验证 |
| C-UX-3 REST 热添加真实验证 | ⏳ 待验证 | `connect_to` 后台触发在本地 loopback 中未充分测试 |
| 72h 长跑测试 | ⏳ 未开始 | v0.3.0 准入线，必须在真实网络下完成 |

---

## 关于云服务器测试的判断

**结论：本周内必须重新联系 Gray-Cloud 进行真实网络测试。**

理由：
1. Bug-1 的根因（incoming 连接状态）与网络方向强相关，WSL2↔Windows 的虚拟交换机行为与真实公网不同
2. 校园网防火墙 ↔ 云服务器安全组 的 NAT/端口映射场景，只有真实部署才能暴露问题
3. Tailscale 穿透在 WSL2 和 Windows 间未验证（之前是用真实 Tailscale IP 穿透的）
4. 如果云服务器不更新，对侧仍运行旧版本，永远测不出修复效果

---

## 云服务器部署方案（对侧无编译环境）

由于 Gray-Cloud 缺少编译硬件基础，提供三个方案：

### 方案 A：WSL2 编译 → 上传（推荐）
```bash
# 在 WSL2 中编译 Linux binary
cargo build --release -p syncthing
# binary 位置：target/release/syncthing
# 用 scp 上传到云服务器
scp target/release/syncthing hadoop@gray-cloud:/home/hadoop/syncthing-rust/
```

### 方案 B：Windows 交叉编译
```bash
# 安装 Linux musl target（静态链接，不依赖云服务器 glibc 版本）
rustup target add x86_64-unknown-linux-musl
cargo build --release -p syncthing --target x86_64-unknown-linux-musl
# binary 可直接在云服务器运行
```

### 方案 C：GitHub Actions 编译 Release
```yaml
# 在 CI 中编译 Linux + Windows binary，上传到 GitHub Releases
# 云服务器直接下载 release asset
```

---

## 下一步行动计划

### 本周（5.15-5.18）
1. **周一晚/周二**：联系 Gray-Cloud，确认对侧是否可接收 binary 或协助编译
2. **周二**：上传/编译最新 main 到云服务器，配置 `syncthing init` 向导生成 config
3. **周三**：执行真实网络双节点测试（校园网 ↔ 云服务器 via Tailscale）
4. **周四**：如测试通过，启动 72h 长跑；如失败，定位修复

### 下周（5.19-5.25）
1. 72h 长跑监控
2. 根据审计报告，启动 unwrap 清除（P1）
3. 完成 C-UX 剩余工作（如有）

---

## 风险点

1. **云服务器无法运行 Rust binary**：可能 glibc 版本不匹配 → 用 musl 静态编译解决
2. **Tailscale 连接不稳定**：校园网 UDP 443 也可能被限流 → 准备 WireGuard 备选
3. **对侧旧数据污染**：Gray-Cloud 上残留旧版 `config.json` / 数据库 → 建议清理后重新 `syncthing init`

---

*报告生成时间：2026-05-15*
*对应 commit：01429a2*
