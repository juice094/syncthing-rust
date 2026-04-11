# Master Agent 验收检查清单

## ⚠️ 验收原则
- **不信任子代理自我报告** - 所有测试通过率需亲自验证
- **代码审查必须逐行** - 检查panic、unwrap、unsafe
- **接口契约强制匹配** - 必须与 syncthing-core traits 兼容
- **编译通过是硬性门槛** - 任何warning都要记录

---

## 验收流程

### 1. 编译验证
```bash
cd C:\Users\22414\Desktop\syncthing-rust-rearch
cargo build --all-features 2>&1 | tee build.log
echo "Exit code: $?"
```

### 2. 静态检查
```bash
cargo clippy --all-targets -- -D warnings 2>&1 | tee clippy.log
cargo fmt --check 2>&1 | tee fmt.log
```

### 3. 接口兼容性检查
- [ ] 所有trait实现都与 `syncthing-core` 定义匹配
- [ ] 类型签名完全一致
- [ ] 无额外/缺失的关联类型

### 4. 代码质量检查
- [ ] 无 `unwrap()` 或 `expect()` 在公共API路径
- [ ] 所有 `unsafe` 代码有文档说明
- [ ] 错误处理使用 `syncthing_core::SyncthingError`
- [ ] 异步函数正确使用 `async_trait`

### 5. 文档检查
- [ ] 每个公共API有文档注释
- [ ] 文件头部有UNVERIFIED标记
- [ ] 复杂逻辑有行内注释

### 6. 单元测试验证
```bash
cargo test --package <crate-name> 2>&1 | tee test-<crate>.log
```
- [ ] 测试实际运行（不是被忽略）
- [ ] 覆盖主要路径
- [ ] 无flaky测试

---

## 各Agent验收重点

### Agent-A (bep-protocol)
- [ ] BepConnection trait完整实现
- [ ] 消息编码/解码正确
- [ ] TLS握手逻辑
- [ ] 协议版本协商

### Agent-B (syncthing-fs)
- [ ] FileSystem trait完整实现
- [ ] 文件监控事件正确触发
- [ ] 块哈希计算正确（SHA-256）
- [ ] 跨平台路径处理

### Agent-D (syncthing-net)
- [ ] Discovery trait完整实现
- [ ] Transport trait完整实现
- [ ] 地址解析正确
- [ ] NAT穿透逻辑

### Agent-E (syncthing-db)
- [ ] BlockStore trait完整实现
- [ ] KV存储操作正确
- [ ] 索引查询性能
- [ ] 块缓存逻辑

---

## 验收结果记录

| Agent | 编译 | Clippy | 测试 | 接口兼容 | 总体 |
|-------|------|--------|------|----------|------|
| A | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| B | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| D | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| E | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |

---

## 不通过处理流程

1. **记录问题** - 具体到文件和行号
2. **分类严重性**
   - 🔴 Critical: 编译失败/接口不匹配
   - 🟠 High: 内存安全/数据竞争风险
   - 🟡 Medium: 代码风格/测试覆盖不足
   - 🟢 Low: 文档缺失/警告
3. **退回修改** - 明确修改要求
4. **重新验收** - 全量重新验证

---

**主会话**: 严格按此清单执行验收，不轻信子代理报告。
