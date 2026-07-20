---
type: Situation Report
title: 项目推进情况说明 — 2026-06-03
description: v0.2.10-rc2 阶段推进情况：Phase 0 完成项、关键修复、云端部署与测试基线。
resource: ./SITUATION_2026-06-03.md
tags: [plan, situation, phase0, okf]
status: archived
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# 项目推进情况说明 — 2026-06-03

## 当前版本: v0.2.10-rc2

## 已完成 (Phase 0 全部)

| # | 项目 | 详情 | 文件变更 |
|---|------|------|----------|
| P0.1 | BEP 协议 | Hello→prost derive (-180行手写); LZ4 写入压缩 | `bep-protocol/`, `syncthing-net/connection/io.rs` |
| P0.2 | 连接生命周期 | ClusterConfig / Ping / Close / DeviceID 验证 (已有, 审计确认) | — |
| P0.3 | API 端点 | pause/resume/scan/override/revert (已有, SyncService trait impl) | — |
| P0.4 | Ignore 模式 | `**/` 任意深度匹配; `//`→`#` 注释 (Go 兼容); `#include` 修复 | `syncthing-fs/ignore/`, `syncthing-sync/ignore.rs` |
| FIX | rename_with_retry | Windows 文件锁三层回退 (remove→rename→指数退避 1s/2s/4s/8s ×5) | `syncthing-sync/puller/mod.rs` |
| FIX | pull 循环 | MIN_PULL_GAP=2s 合并远程索引通知; 日志噪音降 99% | `syncthing-sync/folder_model/mod.rs` |
| FIX | watcher 反馈循环 | `.syncthing.*.tmp` 事件过滤; 5s debounce; 30s 最小 scan 间隔 | `syncthing-sync/folder_model/mod.rs` |
| FIX | scanner 日志 | scanned/new/modified/changed 计数器 | `syncthing-sync/scanner.rs` |
| OPS | 云端部署 | SSH+SCP 部署脚本 `scripts/cloud-deploy.sh`; memory 封存 | 新文件 |
| DOCS | AGENTS.md | 版本/测试计数/已完成项/阻塞项 更新 | `AGENTS.md` |

**测试**: 366 passed, 0 failed, 0 warnings. E2E 双向同步 gray-workspace 574 文件已验证。

## 已验证的链路

```
ROG-X (Windows) ←→ Gray-Cloud (Ubuntu)
  创建文件 → watcher → scanner → DB → index_dispatcher → 云端接收
  全程 ~1 秒
```

## 阻塞项

### B1: rename_with_retry 真实文件锁验证

**状态**: 🟡 代码已部署双端，逻辑正确，但触发条件未满足

**阻塞原因**: 双向同步中的独立 bug — 云端新建文件后，ROG-X 索引中无此文件 → 云端 puller 误判为"远程已删除" → 删除本地副本。测试文件无法抵达 ROG-X，rename 无法触发。

**所需修复**: `conflict_resolver` 或 `index_handler` 中需添加保护 — 本地新建文件（local version > remote 或 remote 无此文件）不应被远程缺失覆盖。该文件应被 push 到对端而非被删除。

**优先级**: P1 — 影响正确性但不阻塞开发主路径

### B2: 连接间歇性断开

**状态**: 🟡 观察到多次 "Connection closed" + 重连 (retry_count=1)，尤其在大量文件传输时

**可能原因**: 云端的 systemd watchdog 与新 binary 竞态; 或 BEP session 在 block 传输超时时断开

**优先级**: P2 — 可在 stress test 中进一步诊断

## 接下来可推进

### 独立任务 (不需解决阻塞项)

| 优先级 | 任务 | 预计 | 说明 |
|--------|------|------|------|
| **P1** | B1 修复: 双向删除误判 | 1-2天 | conflict_resolver 添加本地新建保护 |
| **P1** | rename_with_retry 验证 | 30min | B1 修复后自然触达 |
| **P1** | P0.6 跨实现测试 | 3-5h | Rust↔Go BEP 握手+索引交换自动化 CI |
| **P2** | P1.1 版本控制 (Simple) | 1-2天 | 新 crate + puller 集成 + 测试 |
| **P2** | P1.2 符号链接同步 | 3-5天 | scanner + puller + 平台守卫 |
| **P3** | 72h 耐久测试 | 3天 | 双端 stress_test.rs 后台运行 |

### 建议顺序

```
1. B1 修复 (1-2天) → rename_with_retry 验证 → 闭环
2. P0.6 跨实现测试 (并行，不依赖 B1)
3. 启动 Phase 1
```

## 新增脚本

- `scripts/cloud-deploy.sh` — 一键云端部署 (--full / --compile-only / --deploy-only / --status / --stop)
- SSH 可达: `ssh root@100.127.13.26` (需 Tailscale)
- 陷阱记录: watchdog 对抗 / pkill 自杀 / cargo PATH / GitHub 不可达

## 已知未修复问题

1. `//` 注释行在 `.stignore` 中现在被当作字面模式（Go 兼容改动）。用户的 `.stignore` 有大量 `//` 注释行，虽然当前不起实际过滤作用，但语义上不再是注释。
2. 云端 `/root/dev` 不是 git repo — 源码通过 SCP 同步，无法 `git pull`。需要后续初始化 git。
3. 云端 systemd watchdog 路径固定指向 `syncthing-v0.2.10-rc1` — 更新 binary 时需覆盖同名文件或更新 systemd 配置。
