# 验收标准与验证流程

## 基本原则

> **子代理交付物视为不可靠，必须经过严格验收**

## 验收检查清单

### Phase 1: 编译验收

| 检查项 | 验收命令 | 通过标准 |
|--------|----------|----------|
| Workspace 编译 | `cargo check --workspace` | 0 错误，允许警告 |
| Release 构建 | `cargo build --release` | 成功生成可执行文件 |
| CLI 可执行文件 | `ls target/release/syncthing.exe` | 文件存在且有内容 |
| Demo 可执行文件 | `ls target/release/demo.exe` | 文件存在且有内容 |

**严格标准**:
- ❌ 任何编译错误都不被接受
- ⚠️ 警告需记录但不阻塞验收
- ❌ 不能修改测试代码来通过验收

### Phase 2: 测试验收

| 检查项 | 验收命令 | 通过标准 |
|--------|----------|----------|
| Core 测试 | `cargo test -p syncthing-core` | 全部通过 |
| FS 测试 | `cargo test -p syncthing-fs` | 全部通过 |
| DB 测试 | `cargo test -p syncthing-db` | 全部通过 |
| API 测试 | `cargo test -p syncthing-api` | 全部通过 |
| BEP 测试 | `cargo test -p bep-protocol` | 全部通过 |
| Net 测试 | `cargo test -p syncthing-net` | 全部通过 |
| Sync 测试 | `cargo test -p syncthing-sync` | 全部通过 |

**严格标准**:
- ❌ 任何测试失败都不被接受
- ❌ 跳过测试不被接受
- ✅ 必须实际运行并看到 `test result: ok`

### Phase 3: 功能验收

| 检查项 | 验收命令 | 通过标准 |
|--------|----------|----------|
| CLI init | `./syncthing.exe init` | 成功创建配置 |
| CLI generate | `./syncthing.exe generate` | 输出生成的设备ID |
| CLI scan | `./syncthing.exe scan` | 扫描文件夹无崩溃 |
| Demo 运行 | `./demo.exe --help` | 显示帮助信息 |

## 验收流程

```
子代理完成任务
       ↓
子代理提交交付物
       ↓
主代理独立验证（不信任子代理的输出）
       ↓
运行验收检查清单中的所有命令
       ↓
全部通过 → 验收成功
任何失败 → 退回子代理修复
```

## 不信任项

以下子代理输出**不可信任**，必须独立验证：

1. ❌ "编译成功" 的文本声明
2. ❌ "测试通过" 的文本声明  
3. ❌ 测试数量的声称
4. ❌ 功能可用性的声称

**唯一可信证据**:
- 实际运行的命令输出截图/日志
- 可执行文件的实际存在
- 测试运行的实际结果

## 当前问题清单（修复前）

1. **编译阻塞**: `protobuf-src` 在 Windows 上无法编译
2. **依赖冲突**: rustls 版本不一致（0.21 vs 0.23）
3. **构建脚本**: bep-protocol/build.rs 声明使用预生成代码但依赖 protobuf-src

## 修复策略

### 方案 A: 移除 protobuf-src（推荐）

从 bep-protocol/Cargo.toml 中移除 `protobuf-src` 依赖，因为：
- build.rs 声明使用预生成的代码
- protobuf-src 在 Windows 上无法工作

### 方案 B: 条件编译

仅对非 Windows 平台启用 protobuf-src：
```toml
[target.'cfg(not(windows))'.build-dependencies]
protobuf-src = "1.1"
```

## 验收记录

| Phase | 验收人 | 日期 | 结果 | 备注 |
|-------|--------|------|------|------|
| 编译验收 | 主代理 | - | 待验收 | - |
| 测试验收 | 主代理 | - | 待验收 | - |
| 功能验收 | 主代理 | - | 待验收 | - |
