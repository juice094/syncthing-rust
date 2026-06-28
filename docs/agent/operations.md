---
type: Runbook
title: Build Artifacts, Deployment and Recovery
description: syncthing-rust 的构建产物、部署脚本、灾备恢复协议与 CI/CD 说明。
resource: ./operations.md
tags: [agent, operations, deployment, recovery, ci-cd, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# 部署与运维

---

## 1. 构建产物

- `target/release/syncthing.exe` / `syncthing`：主守护进程
- `target/release/syncthing-cli`：CLI 工具
- `target/release/syncthing-monitor`：监控工具
- `target/release/syncthing-bench`：基准测试
- `target/release/syncthing-mcp-bridge`：MCP Bridge
- `target/release/syncthing-tray.exe`：托盘薄 wrapper（向后兼容，实际逻辑已合并进 `syncthing.exe` 的 `tray` feature）

### 1.1 常用构建命令

```bash
# 编译 release
cargo build --release

# 运行守护进程
cargo run --release -- run --config-dir ~/.syncthing

# 交互式初始化向导
cargo run --release -- init

# 启动 TUI
cargo run --release -- tui --config-dir ~/.syncthing

# 启动 Windows 托盘
cargo run --release -- tray --config-dir ~/.syncthing

# 启动 Relay Server v1
cargo run --release -- relay-server --listen :22067 --tls-cert cert.pem --tls-key key.pem

# CLI 查询状态
cargo run -p syncthing-cli -- status
```

---

## 2. 部署脚本

- `scripts/cloud-deploy.sh`：从 Windows 编译并部署到云端 Ubuntu（SCP + systemd）
- `scripts/72h_stress_test.sh` / `72h_monitor.sh` / `72h_report.sh`：长跑测试
- `scripts/check-health.ps1` / `check-sync-consistency.ps1`：健康检查
- `scripts/recover-remote.sh`：对侧格式化/重装后的灾备恢复
- `scripts/two-node-real-network-test.ps1` / `scripts/stop-two-node-test.ps1`：真实网络双节点测试
- `scripts/generate-sbom.sh`：生成 SBOM（CycloneDX JSON）用于发布审计

---

## 3. Docker 与 Relay 部署

v3.0.4 起提供容器化部署模板：

| 文件 | 用途 |
|:---|:---|
| `deploy/Dockerfile` | 多阶段构建 `syncthing` 镜像 |
| `deploy/docker-compose.relay.yml` | 守护进程 + Relay Server + nginx 组合 |
| `deploy/nginx-relay.conf` | nginx 反向代理 / TLS 终止配置 |
| `deploy/grafana-dashboard.json` | Prometheus 监控仪表板 |

```bash
# 构建镜像
docker build -f deploy/Dockerfile -t syncthing-rust:latest .

# 启动守护进程 + Relay + nginx
docker compose -f deploy/docker-compose.relay.yml up -d
```

详细灾备恢复流程见 `docs/operations/DISASTER_RECOVERY.md`。

---

## 4. 灾备恢复协议（对侧格式化/重装后）

1. 停止双端 syncthing
2. 删除双端 `db/` 和 `syncthing.pid`
3. 本侧 `git bundle create workspace.bundle --all`
4. SCP → 对侧 `git clone workspace.bundle workspace`
5. 对侧 `systemctl start syncthing`
6. 本侧启动 syncthing
7. 验证：0 error code 3 + scans stable（files_changed=0）

---

## 5. CI / CD

- 平台：GitHub Actions（`.github/workflows/ci.yml` + `.github/workflows/release.yml`）
- 矩阵：ubuntu-latest / windows-latest / macos-latest
- CI job（`.github/workflows/ci.yml`）：fmt、clippy（×3 OS）、test（×3 OS）、audit、cargo-deny、all-features、bench-smoke、release-check（×3 OS）、doc-check、e2e-test、file-size（600 行软限制检查）—— 11 个 job 定义，矩阵展开后 17 个运行实例
- Release job（`.github/workflows/release.yml`）：build-linux、build-windows —— 2 个运行实例
- **注意**：`README.md` / `CHANGELOG.md` 声称 19/19 jobs passing（17 CI + 2 release）；本地 `cargo deny check all` 也已通过，CI 状态需以最新 GitHub Actions 运行结果为准。

---

## 6. 默认端口

- BEP：`22001`（避免与 Go Syncthing 默认 `22000` 冲突）
- REST API：`8385`（避免与 Go Syncthing 默认 `8384` 冲突）

---

## 7. 关键默认值

- 扫描间隔：`3600` 秒
- 块大小：`128 KiB`
- TCP keepalive：`60s` 间隔 / `10s` 探测间隔
- BEP 心跳：`30s`
- 连接超时：`120s`
- 最大连接数：`1000`
- 日志轮转：按小时，保留 `168` 个文件（7 天）
