# 子代理交付验收标准

> **严格验收模式：不信任子代理声明，必须独立验证**

## 通用验收规则

1. **编译必须通过**：`cargo check --workspace` 0错误
2. **测试必须通过**：`cargo test -p <crate>` 全部通过
3. **功能必须验证**：实际运行验证，不接受日志声明
4. **代码必须审查**：检查是否有TODO/FIXME/UNVERIFIED标记

## Agent-Sync 验收标准

### 编译验收
```bash
cargo check -p syncthing-sync
cargo test -p syncthing-sync
```

### 功能验收
- [ ] `FolderModel` 实现 `SyncModel` trait
- [ ] 文件扫描后正确生成 `IndexUpdate`
- [ ] 收到远程 `Index` 后能计算需要拉取的文件
- [ ] `pull_loop` 定期执行（每分钟检查）
- [ ] 文件冲突时生成 `.sync-conflict-YYYYMMDD-HHMMSS` 副本
- [ ] 版本向量正确比较文件版本

### 验证命令
```bash
# 运行同步测试
cargo test -p syncthing-sync -- test_pull
cargo test -p syncthing-sync -- test_conflict
```

## Agent-Net 验收标准

### 编译验收
```bash
cargo check -p syncthing-net
cargo test -p syncthing-net
```

### 功能验收
- [ ] `ConnectionManager` 维护活跃连接池
- [ ] 新连接自动触发 `ClusterConfig` 交换
- [ ] 连接断开自动重连（指数退避）
- [ ] TLS 证书正确验证设备ID
- [ ] TCP 22000 端口监听正常

### 验证命令
```bash
# 测试连接
cargo test -p syncthing-net -- test_connection

# 检查端口
netstat -an | grep 22000
```

## Agent-API 验收标准

### 编译验收
```bash
cargo check -p syncthing-api
cargo test -p syncthing-api
```

### 功能验收
- [ ] API 服务在 `syncthing run` 时启动
- [ ] `GET /rest/health` 返回 200
- [ ] `GET /rest/system/status` 返回系统状态
- [ ] `GET /rest/db/status` 返回文件夹状态
- [ ] WebSocket `/rest/events` 推送事件
- [ ] 端口 8384 可访问

### 验证命令
```bash
# 启动服务
./syncthing.exe run &

# 测试API
curl http://127.0.0.1:8384/rest/health
curl http://127.0.0.1:8384/rest/system/status
```

## Agent-Discovery 验收标准

### 编译验收
```bash
cargo check -p syncthing-net
cargo test -p syncthing-net discovery
```

### 功能验收
- [ ] 实现 `Discovery` trait
- [ ] 本地发现 (multicast 21027)
- [ ] 全局发现 (discovery.syncthing.net)
- [ ] 缓存发现结果
- [ ] 定期刷新发现

### 验证命令
```bash
cargo test -p syncthing-net -- test_discovery
```

## 验收流程

```
子代理完成任务
    ↓
提交PR/代码变更
    ↓
主代理独立验证（不阅读子代理的"完成声明"）
    ↓
运行验收检查清单
    ↓
  ├─ 编译通过？
  ├─ 测试通过？
  └─ 功能验证通过？
    ↓
全部通过 → 验收完成
任何失败 → 退回修复
```

## 禁止事项

❌ **子代理不得**：
1. 修改测试代码来让测试通过
2. 添加无法编译的占位代码
3. 声称功能"可用"但未实际测试
4. 引入新的编译错误
5. 破坏现有功能

❌ **主代理不接受**：
1. "应该可以工作"的声明
2. 未经运行的测试日志
3. 部分实现的"骨架"代码
