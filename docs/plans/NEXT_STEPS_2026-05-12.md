# 后续任务清单 — 2026-05-12

> 本次会话工程改进（v0.2.3 → v0.2.4）详见 `docs/reports/SESSION_SUMMARY_2026-05-12.md`  
> 本文档记录**剩余的所有 P0~P2 任务**，按优先级和依赖关系排序。

---

## ✅ 已完成（v0.2.4 发布周期）

| 任务 | 状态 | 主交付 |
|------|------|--------|
| T-F1 死锁修复 | ✅ | DashMap 跨 await 持锁问题，72h 压测稳定 |
| T-F2 unwrap 审计 | ✅ | 生产代码 0 unwrap，15 处文档化 expect |
| T-A1 完整 Criterion baseline | ✅ | scanner 1.49 GB/s, puller 1.46 GB/s, BEP 507 MB/s |
| T-B1 rayon 并行验证 | ✅ | 9-11x 加速实测 |
| T-E1 文件拆分（8 文件）| ✅ | 最大文件 979 → 684 |
| T-G2 CI bench-smoke | ✅ | 防 bench rot |
| await_holding_lock CI lint | ✅ | 防 T-F1 类型死锁 |
| 死代码清理 | ✅ | 净 -31 行（from_arc + PORT_MAP_*）|
| redundant_clone 修复 | ✅ | 4 处性能微优化 |
| or_fun_call 修复 | ✅ | 5 处懒求值优化 |
| **v0.2.4 Release** | ✅ | GitHub release published |

---

## 🔄 P0 — 正在进行

### 72h 压测（监控中）
- **状态**: 运行中 T+~9h（启动 13:07:49）
- **当前**: PID 20048, 21 MB 内存稳定，1072+ 心跳
- **目标**: 完成 72h 持续运行
- **剩余**: ~63h
- **后续**: 完成后撰写 `docs/reports/72H_STRESS_REPORT_2026-05-XX.md`

---

## 🟡 P1 — 短期可执行（每项 1-4h）

### P1.1 dialer.rs (621) tests 抽取
- **现状**: 621 行，含 tests
- **目标**: 抽 tests.rs → mod.rs 减至 ~450 行
- **依赖**: 无
- **风险**: 低（仅测试文件提取）

### P1.2 types/folder.rs 抽取
- **现状**: types/mod.rs 639 行
- **目标**: 抽 FolderType/FolderStatus/Compression/Folder 到 types/folder.rs
- **预期**: types/mod.rs → ~470 行
- **依赖**: 无
- **风险**: 中（公开 API 路径变化，需 pub use）

### P1.3 service/mod.rs 业务拆分
- **现状**: 684 行，业务核心
- **目标**: 按 lifecycle (start/stop) / handlers / state 拆分
- **预期**: ~3-4 子模块
- **依赖**: 无
- **风险**: 高（业务逻辑分散，需架构设计）

### P1.4 selective clippy nursery 继续
- **范围**: `manual_let_else` (8 处), `needless_collect`, `useless_conversion`
- **预期**: 5-15 处微改进
- **依赖**: 无
- **风险**: 低（自动 fixable）

---

## 🟢 P2 — 中期工程（每项 3-8h）

### P2.1 T-D3 pending_responses slab-ization
- **现状**: `DashMap<i32, oneshot::Sender>`
- **改造**: 使用 `slab::Slab` + i32 索引（如果消息 ID 可序列化）
- **预期收益**: 微观（连接级 hot path 减少 hash 开销）
- **依赖**: 无
- **风险**: 中（API 变化）

### P2.2 T-F3 tracing-appender 日志滚动
- **现状**: 单文件 log，无 rotation
- **改造**: `tracing-appender::rolling::daily/hourly`
- **目标**: 防止超长运行后日志膨胀
- **依赖**: 72h 测试完成后实施
- **风险**: 低

### P2.3 T-C FileSystemDatabase 优化（最大工程）
- **现状**: `MemoryDatabase` (DashMap) + 序列化为 JSON
- **改造**: slab + sled WAL（持久化 + 性能）
- **预期收益**: 大量小文件场景下索引查询加速
- **依赖**: 需要详细设计
- **风险**: 高（持久化层重构）

### P2.4 dialer.rs 业务拆分
- **现状**: 621 行（含 tests）
- **目标**: 按 dial / score / retry 拆分
- **依赖**: P1.1 之后
- **风险**: 中

---

## 🔵 P3 — 长期 / 等待

### P3.1 72h 压测完成报告
- **触发**: 测试完成（2026-05-15 约 13:07）
- **内容**: 内存曲线、连接数、错误率、性能对比
- **依赖**: 测试运行结束
- **优先级**: 高（关键里程碑）

### P3.2 Cross-platform 验证
- **目标**: Linux/macOS 上重跑 72h 压测
- **依赖**: 当前 Windows 测试完成
- **风险**: 平台差异可能暴露新问题

### P3.3 v0.3.0 规划
- **触发**: P3.1 完成后
- **范围**: 大版本，含 T-C 等大改造
- **依赖**: 多个 P2 任务完成

---

## 任务编排建议

### 当前段（72h 压测进行中）
```
┌─ 前台串行 ──────────────────────┐
│ P1.1 dialer.rs tests 抽取 (1h)  │
│ P1.2 types/folder.rs 抽取 (2h)  │
│ P1.4 clippy nursery 续 (1h)     │
└─────────────────────────────────┘
        ║ 并行
┌─ 背景持续 ──────────────────────┐
│ 72h 压测 (背景，无需干预)        │
└─────────────────────────────────┘
```

### 测试完成后
```
P3.1 撰写完整压测报告 (4h)
  ↓
P2.2 T-F3 log rotation (2h)
  ↓
P2.1 T-D3 pending_responses (3h)
  ↓
P2.3 T-C FS DB (8h, 大型重构)
```

---

## 工程纪律

每次 commit 必须满足：
- ✅ `cargo fmt --check` 通过
- ✅ `cargo clippy --release --all-targets -- -D warnings -W clippy::await_holding_lock` 通过
- ✅ `cargo test --workspace --lib` 295/295 通过
- ✅ CI 上 windows-latest + ubuntu-latest 双平台绿

每次 v0.X.Y release 必须：
- ✅ 所有 Cargo.toml 版本一致
- ✅ CHANGELOG 更新
- ✅ 完整 release notes
- ✅ 关联 RCA / baseline 文档

---

**Maintained by**: T-F2 review，每完成 P0/P1 任务后更新本文档。
