# 后续任务清单 — 2026-05-11

> 本次会话已完成的工作详见 `docs/reports/UNWRAP_AUDIT_2026-05-11.md` 和最新 git log。  
> 本文档记录**剩余的所有 P0~P2 任务**，按优先级和依赖关系排序。

---

## ✅ 本次会话完成清单（参考）

| 任务 | 状态 | 主提交 |
|------|------|--------|
| 项目说明文件完善（README/CONTRIBUTING/CODE_OF_CONDUCT/CI/PR模板） | ✅ | `f9cf147` |
| v0.2.3 稳定版发布（CI 全绿） | ✅ | `0be98cc` |
| T-A1 Criterion 基准基础设施验证 | ✅ | (3 个 bench 编译通过) |
| T-F2 Batch 1+2 unwrap/expect 修复 | ✅ | `5d32af0`, `ea2d9b1` |
| T-E1 大文件拆分（14 → 4 个 >600 行文件） | ✅ | 10 个 refactor 提交 |
| 72h 压测多实例冲突修复 + 守护脚本升级 | ✅ | `ea2d9b1` |
| CI 修复（fmt + clippy + test 全绿） | ✅ | `1e804bf`, `c50fe89` |
| 项目临时文件清理（26 文件 + 7 目录到回收站） | ✅ | (本次) |

---

## 🟥 P0 — 立即推进

### T-F1 72h 压测恢复
- **现状**: PID 20368 重启后又死亡，仅产生 1 行 CSV 数据
- **诊断方向**:
  - 检查 stress_test.exe 是否有未捕获 panic
  - 增强日志：每 5 分钟主动 `info!("alive at T+{}", elapsed)`
  - 增加 panic hook：`std::panic::set_hook` 写入独立崩溃日志
- **预估工时**: 2 小时
- **依赖**: 无

### T-F2 unwrap/expect 审计（Batch 3-6）
- **现状**: 修复 6 个，剩余 ~724 个（非测试代码）
- **目标**: < 50
- **批次计划**:
  | Batch | Crate | 当前 | 目标 |
  |-------|-------|------|------|
  | 3 | syncthing-test-utils + syncthing-sync | 87 | ~40 |
  | 4 | syncthing-fs | 154 | ~60 |
  | 5 | syncthing-db | 194 | ~80 |
  | 6 | syncthing-net | 240 | ~100 |
- **预估工时**: ~15 小时
- **依赖**: 无

---

## 🟧 P1 — 本周完成

### T-A1 Criterion 基线收集
- **现状**: 3 个 bench 套件编译通过（scanner/puller/encode_decode），但未实际运行
- **行动**:
  ```powershell
  cargo bench -p syncthing-fs --bench scanner
  cargo bench -p syncthing-sync --bench puller
  cargo bench -p bep-protocol --bench encode_decode
  ```
- **产出**: `target/criterion/` HTML 报告 + 提交到 `docs/reports/BASELINE_2026-05-11.md`
- **预估工时**: 30 分钟
- **依赖**: 无

### T-B1 Scanner SHA-256 并行化（基准验证）
- **现状**: 代码已用 rayon 线程池，但未量化收益
- **行动**: T-A1 基线就绪后，对比单线程 vs rayon 并行的吞吐量
- **预期收益**: 2~8x（取决于 CPU 核心数）
- **预估工时**: 2 小时
- **依赖**: T-A1

### T-E1 剩余 4 个大文件优化（可选）
- 当前已达成 ≤4 目标，但仍可进一步拆分：
  - `bep-protocol/src/messages.rs` (910) — 按消息类型拆分到 messages/{hello,index,request,response}.rs
  - `syncthing-core/src/types.rs` (882) — 按领域拆分到 types/{config,folder,device,file}.rs
  - `syncthing-net/src/connection.rs` (770) — 测试代码 76 行，可提取
  - `syncthing-sync/src/service.rs` (715) — 测试代码 32 行，需提取业务逻辑子模块
- **预估工时**: 4 小时
- **依赖**: 无

---

## 🟨 P2 — 本月完成

### T-C FileSystemDatabase 存储优化
- **现状**: per-file JSON，每次读写都是 O(N) syscall
- **行动**: 引入 slab + 内存索引 + sled WAL
- **预估工时**: 8 小时
- **依赖**: T-A1（需要 DB benchmark）

### T-D3 pending_responses slab 化
- **现状**: BEP `Session` 使用 `HashMap<u32, oneshot::Sender>` 管理 pending
- **行动**: 改为 `slab::Slab` 减少哈希查找
- **预估工时**: 3 小时
- **依赖**: T-A1（需要 BEP RTT benchmark）

### T-F3 日志切片（tracing-appender）
- **现状**: 单一 log 文件，可能撑爆磁盘
- **行动**: 集成 `tracing-appender::rolling::daily`
- **预估工时**: 2 小时
- **依赖**: 无

### T-G2 Criterion CI 回归检测
- **现状**: CI 已有 fmt/clippy/test/audit，无 bench
- **行动**: GitHub Actions 加入 `cargo bench --no-fail-fast` + 阈值告警
- **预估工时**: 2 小时
- **依赖**: T-A1 + T-G1（已完成）

---

## 📋 推荐推进顺序

```
今天 ──┬──► T-F1 压测诊断（必须，否则无法验证稳定性）
       └──► T-A1 Criterion 基线（30 分钟快速完成）

明天 ──┬──► T-F2 Batch 3（syncthing-test-utils + syncthing-sync）
       └──► T-B1 Scanner 并行化验证

本周 ──┬──► T-F2 Batch 4-5（syncthing-fs + syncthing-db）
       ├──► T-E1 剩余 4 个文件拆分
       └──► T-F3 日志切片

本月 ──┬──► T-F2 Batch 6（syncthing-net）
       ├──► T-C DB 存储优化
       ├──► T-D3 pending_responses slab 化
       └──► T-G2 Criterion CI 回归检测
```

---

## 📌 长期路线图（参考）

详见 `docs/plans/POST_V0_2_0_ROADMAP.md`：
- **Phase 6**: QUIC 传输层（可选）
- **Phase 7**: Go Syncthing 完整互操作验证
- **Phase 8**: Web GUI（非必须，TUI 已满足核心需求）
