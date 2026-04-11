# Master Agent 并行开发规划

**项目**: Syncthing Rust 重构  
**当前完成度**: 75%  
**目标**: 生产就绪  
**策略**: 并行推进 + 严格验收  

---

## 核心原则

1. **子代理交付 = 不可靠** - 所有代码必须经过主控验证
2. **测试驱动** - 无测试 = 未完成
3. **并行推进** - 独立模块同时开发
4. **强制验收** - 每个里程碑必须达标

---

## 剩余工作分解

### Phase 1: 网络层实现 (最高优先级)

| 任务 | 子代理 | 工作量 | 依赖 | 验收标准 |
|------|--------|--------|------|----------|
| Iroh 连接建立 | Agent-Net-1 | 8h | None | 单元测试 ≥10, 集成测试通过 |
| BEP 消息流集成 | Agent-Net-2 | 8h | Agent-Net-1 | 消息编解码端到端 |
| 设备发现实现 | Agent-Net-3 | 6h | None | DHT 发现测试 |
| 连接管理器 | Agent-Net-4 | 6h | Agent-Net-1 | 并发连接测试 |

**里程碑 1**: 两台设备可以建立连接并交换消息
**验收**: 主控编写集成测试，必须通过

---

### Phase 2: 集成与端到端

| 任务 | 子代理 | 工作量 | 依赖 | 验收标准 |
|------|--------|--------|------|----------|
| Sync + Net 集成 | Agent-Int-1 | 10h | Phase 1 | 文件同步端到端 |
| API + Sync 集成 | Agent-Int-2 | 8h | Phase 1 | REST API 控制同步 |
| 错误处理完善 | Agent-Int-3 | 8h | Phase 1 | 错误注入测试 |
| 配置持久化 | Agent-Int-4 | 6h | None | 配置读写测试 |

**里程碑 2**: 完整同步流程可用
**验收**: 单文件夹双向同步测试通过

---

### Phase 3: 测试与优化

| 任务 | 子代理 | 工作量 | 依赖 | 验收标准 |
|------|--------|--------|------|----------|
| 压力测试 | Agent-Test-1 | 10h | Phase 2 | 1000文件同步 |
| 性能基准 | Agent-Test-2 | 8h | Phase 2 | 性能报告 |
| 长时间测试 | Agent-Test-3 | 12h | Phase 2 | 24h 稳定运行 |
| 兼容性测试 | Agent-Test-4 | 8h | Phase 2 | 与 Go Syncthing 互通 |

**里程碑 3**: 生产就绪
**验收**: 所有测试通过，性能达标

---

## 验收规范 (强制执行)

### 代码验收

```rust
//! 每个交付文件必须包含:
//! Task: <任务ID>
//! Agent: <子代理ID>
//! Status: UNVERIFIED
//! Committed: false
//!
//! ⚠️ 此代码未经主控验证，禁止合并
```

### 测试验收标准

| 级别 | 要求 | 验收人 |
|------|------|--------|
| 单元测试 | ≥80% 覆盖率 | 子代理自测 |
| 集成测试 | 主控编写，必须通过 | 主控 |
| 端到端测试 | 真实场景通过 | 主控 |
| 性能测试 | 达到基准 | 主控 |

### 验收检查清单

- [ ] 代码编译无警告 (`cargo check --warnings`)
- [ ] 单元测试通过 (`cargo test`)
- [ ] 文档测试通过 (`cargo test --doc`)
- [ ] Clippy 无警告 (`cargo clippy -- -D warnings`)
- [ ] 格式化正确 (`cargo fmt --check`)
- [ ] 无 `unsafe` 代码 (除非特别批准)
- [ ] 公共 API 有文档
- [ ] 集成测试通过 (主控编写)

---

## 并行调度计划

### Wave 1 (立即启动)

```
Agent-Net-1 ──┬──> Agent-Net-2
              │
Agent-Net-3 ──┤
              │
Agent-Net-4 ──┘
```

所有网络层子代理并行工作

### Wave 2 (Wave 1 验收后)

```
Agent-Int-1 ──┬──> Agent-Int-2
              │
Agent-Int-3 ──┤
              │
Agent-Int-4 ──┘
```

### Wave 3 (Wave 2 验收后)

测试子代理并行执行

---

## 交付物管理

### 目录结构

```
deliverables/
├── wave-1/
│   ├── agent-net-1/         # 子代理工作目录
│   ├── agent-net-2/
│   ├── agent-net-3/
│   └── agent-net-4/
├── wave-2/
│   └── ...
└── verified/                # 主控验证通过后移动至此
```

### 交付状态流转

```
子代理开发
    ↓
[UNVERIFIED] 提交到 deliverables/wave-X/
    ↓
主控验收测试
    ↓
┌─────────┬──────────┐
│  通过   │   失败   │
↓         │          ↓
[VERIFIED]│    退回重写
↓         │
合并到主分支
```

---

## 子代理通信协议

### 任务分配格式

```json
{
  "task_id": "NET-001",
  "agent_id": "Agent-Net-1",
  "type": "IMPLEMENTATION",
  "priority": "P0",
  "deliverables": [
    "crates/syncthing-net/src/connection.rs",
    "tests/"
  ],
  "constraints": {
    "max_lines": 500,
    "no_unsafe": true,
    "test_coverage": 0.8
  },
  "acceptance_criteria": [
    "cargo test passes",
    "clippy clean",
    "integration test passes"
  ]
}
```

### 交付报告格式

```json
{
  "task_id": "NET-001",
  "agent_id": "Agent-Net-1",
  "status": "DELIVERED",
  "timestamp": "2026-04-03T20:00:00Z",
  "files": [
    {
      "path": "src/connection.rs",
      "lines": 180,
      "status": "UNVERIFIED"
    }
  ],
  "tests": {
    "total": 10,
    "passed": 10
  },
  "known_issues": []
}
```

---

## 风险管控

### 高风险任务

| 风险 | 缓解措施 |
|------|----------|
| Iroh API 变更 | 锁定版本 0.32 |
| 子代理超时 | 4小时检查点 |
| 集成失败 | 每日集成测试 |
| 性能不达标 | 早期基准测试 |

### 中止条件

- 子代理 8 小时无响应 → 重新分配
- 连续 3 次验收失败 → 重构或换方案
- 集成测试连续失败 → 暂停开发，修复架构

---

## 时间线

| 阶段 | 预计时间 | 里程碑 |
|------|----------|--------|
| Wave 1 | 2 天 | 网络层可用 |
| Wave 1 验收 | 1 天 | 集成测试通过 |
| Wave 2 | 3 天 | 端到端同步 |
| Wave 2 验收 | 1 天 | 稳定性验证 |
| Wave 3 | 3 天 | 性能达标 |
| 最终验收 | 1 天 | 生产就绪 |

**总计**: ~11 个工作日 (2-3 周)

---

## 立即行动

1. **启动 Wave 1 子代理** (并行)
   - Agent-Net-1: Iroh 连接
   - Agent-Net-2: BEP 消息流
   - Agent-Net-3: 设备发现
   - Agent-Net-4: 连接管理器

2. **准备验收环境**
   - 集成测试框架
   - 验收脚本
   - 测试数据

3. **建立监控**
   - 子代理进度跟踪
   - 代码质量监控
   - 测试覆盖率追踪

---

**规划制定**: Master Agent  
**状态**: READY TO EXECUTE  
**下一步**: 启动 Wave 1 子代理
