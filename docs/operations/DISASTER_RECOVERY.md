# Disaster Recovery Runbook

> **适用于**: syncthing-rust v3.0.4+  
> **目标 RPO**: ≤ 1 小时（配置/证书）+ ≤ 24 小时（同步数据，取决于对端可用性）  
> **目标 RTO**: ≤ 30 分钟（单节点恢复）+ ≤ 2 小时（完整灾备重建）

---

## 1. 备份策略

### 1.1 需要备份的内容

| 资产 | 路径 | 备份频率 | 说明 |
|---|---|---|---|
| TLS 证书 + 私钥 | `{config_dir}/cert.pem`, `key.pem` | 一次性（生成后立即备份） | 丢失后 Device ID 变更，所有对端需重新授权 |
| 配置文件 | `{config_dir}/config.json` | 每次修改后 | 含 API key、设备列表、文件夹配置 |
| 数据库 | `{config_dir}/db/` | 每日 | 文件索引、块缓存 |
| 同步数据 | `{folder.path}` (各同步目录) | 每日增量 | 文件内容本身，可通过 P2P 从对端恢复 |
| .stignore 文件 | `{folder.path}/.stignore` | 每次修改后 | 排除规则 |

### 1.2 备份命令

```bash
# Linux/macOS — 使用 tar + 加密备份
CONFIG_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/syncthing-rust"
BACKUP_DIR="/backup/syncthing-rust/$(date +%Y%m%d)"

mkdir -p "$BACKUP_DIR"

# 1. 证书（最关键 — 丢失后无法恢复）
cp "$CONFIG_DIR/cert.pem" "$CONFIG_DIR/key.pem" "$BACKUP_DIR/"
chmod 600 "$BACKUP_DIR/key.pem"

# 2. 配置
cp "$CONFIG_DIR/config.json" "$BACKUP_DIR/"

# 3. 数据库（需要先停止 daemon）
# systemctl stop syncthing-rust
tar czf "$BACKUP_DIR/db-backup.tar.gz" -C "$CONFIG_DIR" db/

# 4. .stignore 文件
find /data/sync -name ".stignore" -exec cp --parents {} "$BACKUP_DIR/" \;

# 加密备份（推荐用于异地存储）
gpg --encrypt --recipient admin@example.com "$BACKUP_DIR/key.pem"
gpg --encrypt --recipient admin@example.com "$BACKUP_DIR/config.json"
```

```powershell
# Windows — 使用 7-Zip 加密备份
$ConfigDir = "$env:LOCALAPPDATA\syncthing-rust"
$BackupDir = "D:\backup\syncthing-rust\$(Get-Date -Format 'yyyyMMdd')"

New-Item -ItemType Directory -Force -Path $BackupDir

# 证书 + 配置
Copy-Item "$ConfigDir\cert.pem", "$ConfigDir\key.pem", "$ConfigDir\config.json" $BackupDir

# 数据库
net stop syncthing-rust
7z a -p "$BackupDir\db-backup.7z" "$ConfigDir\db\"

# 加密
gpg --encrypt --recipient admin@example.com "$BackupDir\config.json"
```

---

## 2. 恢复流程

### 2.1 场景 A：配置文件损坏/丢失（最常见）

**症状**: 启动报错 `failed to read config` 或 `failed to parse config`

**恢复步骤**:
```bash
# 1. 停止 daemon
systemctl stop syncthing-rust

# 2. 从备份恢复
cp /backup/syncthing-rust/20260101/config.json ~/.local/share/syncthing-rust/config.json
cp /backup/syncthing-rust/20260101/cert.pem ~/.local/share/syncthing-rust/cert.pem
cp /backup/syncthing-rust/20260101/key.pem ~/.local/share/syncthing-rust/key.pem

# 3. 重启
systemctl start syncthing-rust

# 4. 验证
syncthing status
```

### 2.2 场景 B：证书丢失 / 系统重装

**症状**: 设备 ID 变更，所有已配对设备显示"未授权"

**恢复步骤**:
```bash
# 1. 从备份恢复证书文件（优先）
cp /backup/syncthing-rust/<date>/cert.pem {config_dir}/cert.pem
cp /backup/syncthing-rust/<date>/key.pem {config_dir}/key.pem
systemctl start syncthing-rust

# 2. 如无证书备份 — 生成新证书并重新授权
syncthing run  # 自动生成新证书
# 记下新的 Device ID，在所有对端设备上更新授权
```

### 2.3 场景 C：数据库损坏（§15 级联删除防护）

**症状**: 启动报错、文件被意外删除、大量 error code 3

**恢复步骤**:
```bash
# 1. 停止双端 syncthing
systemctl stop syncthing-rust  # 本端
ssh peer systemctl stop syncthing  # 对端

# 2. 删除双方数据库
rm -rf {config_dir}/db/
rm -f {config_dir}/syncthing.pid
ssh peer "rm -rf /var/lib/syncthing/db/"

# 3. 本端重建（从磁盘文件重新索引）
systemctl start syncthing-rust
# daemon 启动后自动执行全量扫描，重新构建索引

# 4. 对端启动
ssh peer "systemctl start syncthing"
# 对端收到本端的全量 Index 后重新同步

# 5. 验证
syncthing status --json | jq '.folders[].globalBytes'
# 确认 globalBytes 匹配实际文件大小
```

### 2.4 场景 D：完整灾备（服务器完全重建）

**前提**: 至少有一个对端设备在线且拥有完整数据

```bash
# 1. 新服务器：安装 syncthing-rust
cargo install --path .  # 或使用预编译二进制

# 2. 恢复证书（如有备份）
mkdir -p ~/.local/share/syncthing-rust
cp /backup/syncthing-rust/<date>/cert.pem ~/.local/share/syncthing-rust/
cp /backup/syncthing-rust/<date>/key.pem ~/.local/share/syncthing-rust/

# 3. 恢复配置
cp /backup/syncthing-rust/<date>/config.json ~/.local/share/syncthing-rust/

# 4. 启动 — 从对端自动同步数据
syncthing run
# 索引交换 → 块请求 → 文件下载完成

# 5. 验证完整性
diff -r /data/sync/ /backup/verify-snapshot/  # 或使用 checksum
```

---

## 3. 数据库维护

### 3.1 压缩/清理

sled 数据库采用 append-only 设计，长期运行后文件可能增长。

```bash
# 重建数据库（清空后重新扫描）
systemctl stop syncthing-rust
rm -rf {config_dir}/db/
systemctl start syncthing-rust
# daemon 自动全量扫描所有文件夹，重建索引
# 注意：索引重建期间同步暂停
```

### 3.2 健康检查

```bash
# 检查 DB 文件大小
du -sh {config_dir}/db/

# 检查索引文件数与实际磁盘文件数是否匹配
syncthing status --json | jq '.folders[].localFiles'
find /data/sync -type f | wc -l
```

---

## 4. Relay Server 灾备

### 4.1 备份

```bash
# Relay server 无状态 — 仅需备份 TLS 证书
cp {config_dir}/cert.pem {config_dir}/key.pem /backup/relay/
```

### 4.2 恢复

```bash
# 更换服务器 IP 后更新 DNS 或客户端配置
syncthing relay-server --listen 0.0.0.0:22067 --session-port 22068
```

---

## 5. 灾备演练检查清单

| 步骤 | 命令 | 验证标准 |
|---|---|---|
| 1. 停止 daemon | `systemctl stop syncthing-rust` | 进程已退出 |
| 2. 备份数据库 | `tar czf backup.tar.gz db/` | 文件已生成 |
| 3. 模拟故障 | `rm -rf db/ config.json` | 文件已删除 |
| 4. 恢复证书 | `cp backup/cert.pem ...` | Device ID 不变 |
| 5. 恢复配置 | `cp backup/config.json ...` | 启动无报错 |
| 6. 启动 daemon | `systemctl start syncthing-rust` | 392 tests baseline |
| 7. 验证同步 | `syncthing status --json` | localFiles == globalFiles |
| 8. 验证完整性 | `diff -r /data/sync/ <snapshot>` | 无差异 |

---

## 6. 故障排除速查表

| 症状 | 可能原因 | 解决 |
|---|---|---|
| `Connection closed` 频繁 | 防火墙阻断 TCP 22001 | 使用 relay-server 或 Tailscale |
| `error code 3 (NoSuchFile)` | DB 索引过期 | 重建数据库 (场景 C) |
| Device ID 变更 | 证书丢失 | 恢复证书备份 (场景 B) |
| `Relay full` | relay 连接数超限 (1000) | 扩容或分流 relay |
| `Failed to set permissions` | Windows/NTFS 不支持 Unix 权限 | 忽略 — 仅 cosmetic |
| `max_connections` 拒绝连接 | 连接数达上限 | 增大 `config.max_connections` |
