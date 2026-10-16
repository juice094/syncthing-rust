---
type: Policy
title: Engineering Discipline
description: syncthing-rust 工程纪律规范：部署测试、代码审查、文档同步、事故复盘与合并门禁。
resource: ./ENGINEERING_DISCIPLINE.md
tags: [policy, engineering, discipline, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# 工程纪律规范

> **生效日期**：2026-05-15  
> **制定背景**：DUAL_NODE_TEST_2026-05-15 耗时 3.5 小时仍未完成 Block Transfer，暴露系统性工程能力缺失。  
> **适用范围**：所有提交到 `main` 分支的代码、配置、文档变更。  

---

## 1. 部署测试纪律（Deployment Testing Discipline）

### 1.1 真实网络测试为 P0 门禁

**任何涉及以下模块的变更，禁止直接合并到 `main`，必须先通过双节点真实网络验证：**

- `syncthing-net` — 连接管理、TLS、BEP session、传输层
- `syncthing-sync` — scanner、puller、watcher、index 处理
- `syncthing-api` — REST endpoint、config 序列化/反序列化
- `cmd/syncthing` — 启动流程、daemon runner、TUI

**验证定义**：
- 至少两个物理隔离节点（或 VM + 物理机，或 WSL2 + Windows 主机）
- 使用非 `127.0.0.1` 的网络路径（即使是局域网也可，禁止单机 loopback）
- 完成完整 BEP 链路：TLS → Hello → ClusterConfig → Index → Block Request/Response → 文件落盘
- 双向同步各至少 1 个文件，校验 SHA-256 一致

### 1.2 Windows 为一级目标平台

**禁止将 Windows 视为“边缘平台”或“后续兼容”。**

- 所有网络/IO/路径相关变更必须在 **Windows 原生环境**（非 WSL2）验证
- CI 必须包含 `windows-latest` runner（当前缺失，为 P0 基础设施债）
- 路径处理禁止使用平台原生分隔符硬编码，必须使用 `std::path::Path` 或统一正斜杠

### 1.3 配置即代码，变更即审查

`config.json` 结构、字段名、默认值、序列化格式的任何变更必须：

1. 更新 `docs/CONFIG_SCHEMA.md`（如不存在则创建；当前待生成）
2. 提供迁移脚本或向后兼容 shim
3. 在 PR 描述中显式列出“用户需要如何修改现有配置”
4. 禁止静默变更默认值（如 `rescan_interval_secs` 从 3600 改 10 必须文档化）

---

## 2. 提交前检查清单（Pre-Commit Checklist）

维护者合并前必须确认以下检查项（建议作为 PR template）：

```markdown
## Pre-Commit Checklist

- [ ] **单元测试**：`cargo test --workspace` 全绿（309/309）
- [ ] **编译平台**：`cargo build --release` 在 Linux + Windows 均通过
- [ ] **真实网络**：如修改 net/sync/api，提供双节点测试录屏或日志片段
- [ ] **Windows 验证**：如修改路径/IO，提供 Windows 本机测试结果
- [ ] **配置兼容**：如修改 config 结构，提供旧配置加载测试
- [ ] **日志审查**：新增代码无 `unwrap()` / `panic!` / `unreachable!`，所有错误路径有 `error!` 日志
- [ ] **文档同步**：`README.md` / `KNOWN_ISSUES.md` / `CHANGELOG.md` 已更新
```

---

## 3. 缺陷管理纪律

### 3.1 发现即登记

任何在测试、代码审查、生产运行中发现的缺陷，**必须在 30 分钟内**登记到 `docs/KNOWN_ISSUES.md`，格式遵循该文档的协作约定。

**禁止**：
- 口头告知缺陷但不登记
- 在代码中 `#[ignore]` 测试而不在文档说明
- 用 "TODO: fix later" 注释代替正式登记

### 3.2 根因分析模板

发现阻塞性缺陷（P0/P1）时，必须在 Git commit message 或 issue 中填写：

```
症状：观察到的行为
复现：100% 复现的步骤
根因：代码层面的具体位置（文件:行号）
影响：用户视角的后果
修复：具体改动
验证：如何证明修复有效
教训：如何避免同类缺陷
```

---

## 4. 自动化与基础设施

### 4.1 CI 必须覆盖

| 检查项 | 优先级 | 当前状态 |
|--------|--------|----------|
| `cargo test --workspace` on ubuntu-latest | P0 | ✅ 已配置 |
| `cargo test --workspace` on windows-latest | P0 | ❌ 缺失 |
| `cargo clippy -- -D warnings` | P1 | ⏳ 待配置 |
| `cargo fmt --check` | P1 | ⏳ 待配置 |
| 双节点 e2e 测试（loopback） | P0 | ✅ `e2e_sync.rs` |
| 双节点 e2e 测试（真实网络 / Tailscale） | P1 | ❌ 缺失，需基础设施 |

### 4.2 发布流程

禁止直接从本地 `cargo publish` 或手动打 tag。

**发布必须**：
1. 从 `main` 切出 `release/vX.Y.Z` 分支
2. 在该分支上运行完整测试矩阵（Linux + Windows + 真实网络）
3. 更新 `CHANGELOG.md` 和 `KNOWN_ISSUES.md`
4. 由第二个维护者审查并合并
5. GitHub Actions 自动打 tag 并发布 release binary

---

## 5. 配置 UX 专项纪律

鉴于 2026-05-15 测试 50% 时间消耗在配置，特设专项纪律：

### 5.1 禁止手工编辑 JSON 作为唯一配置方式

所有面向用户的配置变更必须同时提供：
- **CLI 命令**：如 `syncthing-rust device add --id XXX --addr tcp://host:port`
- **REST API**：`PUT /rest/config/devices` 等端点
- **降级方案**：手工编辑 `config.json` 仍为可能，但必须有 JSON Schema 验证

### 5.2 启动时快速失败（Fail Fast）

启动流程必须在做任何网络监听前完成配置验证：

```rust
// 伪代码示例
fn validate_config(cfg: &Config) -> Result<()> {
    for folder in &cfg.folders {
        ensure_path_exists(&folder.path)?;
        ensure_folder_id_valid(&folder.id)?;
    }
    for device in &cfg.devices {
        ensure_device_id_format(&device.id)?;
        ensure_address_resolvable(&device.addresses)?;
    }
    ensure_no_duplicate_local_device_id(&cfg)?;
    Ok(())
}
```

**错误信息必须人类可读**，禁止直接暴露序列化错误给终端用户。

### 5.3 单实例锁强制

启动时必须获取单实例锁，失败时立即退出并提示：

```
Error: Another syncthing-rust instance is already running (PID 1234).
       If you want to run multiple instances, use --config-dir to separate configurations.
```

---

## 6. 日志与调试纪律

### 6.1 日志级别规范

| 级别 | 使用场景 | 示例 |
|------|----------|------|
| `ERROR` | 用户需要立即处理的问题 | 配置无效、磁盘满、TLS 证书过期 |
| `WARN` | 可能影响功能但可自愈 | 连接断开（会重试）、watcher 回退到轮询 |
| `INFO` | 生命周期事件 | 服务启动、文件夹扫描完成、连接建立 |
| `DEBUG` | 开发调试 | 单个 BEP 帧内容、块请求详情 |
| `TRACE` | 高频内部状态 | 每个 `tokio::select!` 分支触发 |

**禁止**：
- 在热路径使用 `info!`（如每收到一个网络包都 info）
- 使用 `println!` 代替 `tracing` 日志

### 6.2 调试支持

任何涉及网络/同步的模块必须提供 `debug` 或 `inspect` 子命令/端点：

- `GET /rest/system/connections` — 当前连接列表、每个连接的 `is_alive` 状态、最后活动时间
- `GET /rest/system/status` — 各文件夹的 scan/pull/watcher 任务状态
- `syncthing-rust inspect blocks --folder test-folder` — 待拉取的块队列

---

## 7. 代码审查红线

以下代码在 PR 中一经发现，**必须阻止合并**：

1. **新的 `unwrap()` / `expect()` / `panic!` / `unreachable!()`** 在生产代码路径（测试代码除外）
2. **新的 `unbounded_channel()`** 无界 channel（测试代码除外）
3. **平台特定路径硬编码**（如 `"/tmp/"`、`"C:\\"`）
4. **无对应单测的修复** — “这个改动太简单了不需要测试” 不是理由
5. **配置结构变更无文档更新**
6. **Windows 相关变更无 Windows 测试证据**

---

## 8. 违规处理

本规范由项目维护者强制执行。

- **首次违规**：PR 退回，要求在描述中补充缺失项
- **重复违规**：暂停该贡献者的直接推送权限，改为 PR 必须经过第二人 review
- **重大事故**（如导致 3.5 小时测试失败的配置缺陷）：召开 15 分钟复盘会，输出 `docs/postmortems/YYYY-MM-DD_标题.md`

---

## 9. 相关文档

- `docs/KNOWN_ISSUES.md` — 缺陷登记
- `docs/reports/DUAL_NODE_TEST_2026-05-15.md` — 本规范制定的直接原因
- `docs/archive/plans/NEXT_STEPS_2026-05-15.md` — 修复计划与路线图（已归档）

---

**签署**：

本规范经 2026-05-15 项目复盘会议讨论通过，即日起生效。所有后续提交视为已阅读并同意遵守。
