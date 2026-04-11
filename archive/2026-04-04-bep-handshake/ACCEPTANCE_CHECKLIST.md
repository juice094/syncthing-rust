# 强制验收检查清单

**适用范围**: 所有子代理交付  
**执行者**: Master Agent (严禁子代理自审)  
**状态**: ACTIVE

---

## 第一阶段: 代码质量检查

### 1.1 编译检查
```bash
cd C:\Users\22414\Desktop\syncthing-rust-rearch
cargo check -p <package>
```
- [ ] 编译无错误
- [ ] 警告 ≤ 5 个 (可接受范围)
- [ ] 无 `unsafe` 代码 (除非预先批准)

### 1.2 格式化检查
```bash
cargo fmt -- --check
```
- [ ] 代码格式正确

### 1.3 Clippy 检查
```bash
cargo clippy -p <package> -- -D warnings
```
- [ ] 无 Clippy 警告

---

## 第二阶段: 测试检查

### 2.1 单元测试
```bash
cargo test -p <package>
```
- [ ] 所有测试通过
- [ ] 新增测试 ≥ 要求数量
- [ ] 测试覆盖率 ≥ 80%

### 2.2 文档测试
```bash
cargo test --doc -p <package>
```
- [ ] 所有文档示例通过

### 2.3 验收测试
```bash
cargo test --test net_acceptance
```
- [ ] 对应验收测试通过

---

## 第三阶段: 代码审查

### 3.1 代码规范
- [ ] 所有公共 API 有文档注释
- [ ] 无 `unwrap()` / `expect()` (生产代码)
- [ ] 错误处理使用 `SyncthingError`
- [ ] 异步函数使用 `async_trait`
- [ ] 无死代码 (dead_code 允许标记除外)

### 3.2 接口合规
- [ ] 实现了所有要求的 trait 方法
- [ ] 函数签名与 trait 定义一致
- [ ] 返回类型正确

### 3.3 安全性
- [ ] 无 Panic 路径 (除非确实不可恢复)
- [ ] 资源正确释放 (Drop 实现)
- [ ] 无资源泄漏

---

## 第四阶段: 功能验证

### 4.1 功能测试
根据任务要求验证:
- [ ] 核心功能正常工作
- [ ] 边界条件处理正确
- [ ] 错误场景处理正确

### 4.2 集成测试
- [ ] 与现有模块集成无问题
- [ ] 接口契约遵守

---

## 验收结果

### 通过
如果所有检查项通过:
1. 标记为 `VERIFIED`
2. 合并到主分支
3. 更新 STATUS.md

### 失败
如果有任何检查项失败:
1. 标记为 `FAILED`
2. 创建问题清单
3. 退回子代理修复
4. 重新提交后再次验收

---

## 验收记录模板

```markdown
## 验收记录: <Task-ID>

**Agent**: <Agent-ID>  
**日期**: YYYY-MM-DD  
**验收人**: Master Agent

### 结果: [PASS / FAIL]

### 检查项状态
- 编译: [PASS / FAIL]
- 测试: [PASS / FAIL] (X/Y passed)
- 代码审查: [PASS / FAIL]
- 功能验证: [PASS / FAIL]

### 问题清单 (如果 FAIL)
1. 问题描述
2. 期望结果
3. 实际结果

### 行动
- [ ] 通过，合并到主分支
- [ ] 失败，退回修复
```

---

**注意**: 此清单必须严格执行，无例外。
