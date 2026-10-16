---
type: Runbook
title: Gray-Cloud Operations Manual
description: Gray-Cloud（Ubuntu 24.04）节点的 syncthing-rust 部署、运维、灾备与真实网络双节点验证 SOP。
resource: ./GRAY_CLOUD_OPS_MANUAL.md
tags: [ops, manual, gray-cloud, runbook, okf]
status: active
project: syncthing-rust
timestamp: 2026-06-25T00:00:00Z
---

# Gray-Cloud 操作手册 — syncthing-rust

> **生成时间**：2026-05-15
> **最后验证版本**：v3.0.3（2026-06-13）
> **适用对象**：Gray-Cloud Linux 节点运维
> **Windows 侧 Device ID**：以本侧 `cert.pem` / `syncthing-cli show-id` 输出为准（init wizard 打印值仅供参考）

---

## 1. 项目状态简报（致格雷）

宿与格雷于 2026-05-15 晚间完成 **真实网络双节点端到端文件同步验证**。

**验证结果**：✅ **全部通过**
- TCP 三次握手（Tailscale 虚拟网络）
- TLS 1.3 证书校验
- BEP Hello / ClusterConfig
- Index 双向交换
- Block Request/Response（块传输）
- 文件落盘（双向）

**测试文件**：
| 方向 | 文件名 | 大小 | 校验 |
|------|--------|------|------|
| Windows → Linux | `test-from-windows.txt` | 88 bytes | ✅ |
| Linux → Windows | `test-from-linux.txt` | 51 bytes | ✅ |

---

## 2. 环境确认清单

请格雷在执行任务前确认以下配置：

```bash
# 检查守护进程
pgrep -f syncthing
# 应返回 PID，如 1096414

# 检查监听端口
ss -tlnp | grep 22001
# 应显示 0.0.0.0:22001

# 检查配置目录
ls ~/syncthing-test/config.json

# 检查 Device ID 配置
grep "4FXSKHU" ~/syncthing-test/config.json
# 应出现 2 处（devices 和 folders.devices）

# 检查同步目录
ls ~/syncthing-test/sync/
```

**若以上任何一项异常，请先修复后再执行任务。**

---

## 3. 已知缺陷与规避方法

| ID | 缺陷 | 影响 | 规避方法 |
|----|------|------|---------|
| D-1 | Scanner 不会自动排除元数据文件 | `config.json`、`cert.pem`、`db/`、`logs/` 会被同步到对侧 | **执行 Task 1 前，在 sync 目录创建 `.stignore`**（见 §4.1） |
| D-2 | 证书覆盖 config.json 中的 local_device_id | 若手动改 ID，重启后会被证书覆盖 | **始终以证书为准确认 Device ID**，不要手动修改 `local_device_id` |
| D-3 | Puller 对缺失文件触发 NoSuchFile | 若索引存在但文件被删，会报错 | 避免在同步过程中手动删除文件；若发生，重启即可恢复 |
| D-4 | REST API 部分端点空引用异常 | `/rest/system/connections` 可能 500 | 使用 `netstat` 和日志代替 API 查询连接状态 |

---

## 4. 可执行自动化任务

### Task 1：部署 `.stignore`（必须先执行）

在格雷侧 sync 目录创建 `.stignore`，防止元数据污染：

```bash
cat > ~/syncthing-test/sync/.stignore <> 'IGNOREEOF'
// Syncthing 元数据自动排除
.stfolder
.stignore
.stversions
*.syncthing.tmp

// 配置文件（若配置目录与同步目录重合时）
config.json
cert.pem
key.pem
syncthing.pid

// 数据库与日志
db/**
logs/**
*.log

// 系统文件
*.tmp
~*
IGNOREEOF
```

**验证**：创建后等待 30 秒，确认 `db.syncthing.tmp`、`config.syncthing.tmp` 等不再出现在 sync 目录。

---

### Task 2：72h 耐久测试（稳定性门控）

**目标**：验证当前版本在持续运行 72 小时内的连接稳定性和内存泄漏。

**脚本**：`~/syncthing-test/endurance_test.sh`

```bash
#!/bin/bash
# endurance_test.sh — 72h 耐久测试

TEST_DIR="$HOME/syncthing-test/sync/endurance"
mkdir -p "$TEST_DIR"

LOG_FILE="$HOME/syncthing-test/logs/endurance_$(date +%Y%m%d_%H%M%S).log"
echo "=== 72h 耐久测试开始: $(date) ===" > "$LOG_FILE"

# 每 5 分钟生成一个测试文件，持续 72h
for i in $(seq 1 864); do
    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    FILENAME="endurance_${TIMESTAMP}_${i}.txt"
    CONTENT="Endurance test round $i at $TIMESTAMP. PID=$(pgrep -f syncthing)."

    echo "$CONTENT" > "$TEST_DIR/$FILENAME"
    echo "[$(date)] Created $FILENAME" >> "$LOG_FILE"

    # 每 10 轮检查一次对侧是否收到
    if [ $((i % 10)) -eq 0 ]; then
        sleep 30
        RECEIVED=$(find "$TEST_DIR" -name "*windows*" -type f 2>/dev/null | wc -l)
        echo "[$(date)] Windows files received: $RECEIVED" >> "$LOG_FILE"

        # 内存检查
        MEM=$(ps -o rss= -p $(pgrep -f syncthing) 2>/dev/null)
        echo "[$(date)] syncthing RSS: ${MEM}KB" >> "$LOG_FILE"
    fi

    sleep 300  # 5 分钟
done

echo "=== 72h 耐久测试结束: $(date) ===" >> "$LOG_FILE"
```

**执行方式**：
```bash
chmod +x ~/syncthing-test/endurance_test.sh
nohup ~/syncthing-test/endurance_test.sh > /dev/null 2>&1 &
echo $! > ~/syncthing-test/endurance.pid
```

**监控指标**：
- 连接是否保持 ESTABLISHED（`ss -tn | grep 100.73.228.59`）
- 内存 RSS 是否持续增长（泄漏）
- 文件是否双向同步无丢失

---

### Task 3：大文件传输压测

**目标**：测试 Block Transfer 在 100MB / 1GB 文件下的性能和稳定性。

**脚本**：`~/syncthing-test/benchmark_large_file.sh`

```bash
#!/bin/bash
# benchmark_large_file.sh — 大文件压测

TEST_DIR="$HOME/syncthing-test/sync/benchmark"
mkdir -p "$TEST_DIR"

SIZES=("10M" "100M" "500M")

for SIZE in "${SIZES[@]}"; do
    FILENAME="benchmark_${SIZE}.bin"
    echo "[$(date)] Generating $FILENAME ($SIZE)..."
    dd if=/dev/urandom of="$TEST_DIR/$FILENAME" bs=1M count=${SIZE%M} status=progress

    echo "[$(date)] $FILENAME created. Waiting for Windows side to sync..."
    sleep 60

    # 检查对侧是否回传确认文件
    if [ -f "$TEST_DIR/${FILENAME}.received" ]; then
        echo "[$(date)] $FILENAME sync confirmed by Windows."
        rm "$TEST_DIR/${FILENAME}.received"
    else
        echo "[$(date)] WARNING: $FILENAME sync not confirmed within 60s."
    fi
done
```

**Windows 侧配合脚本**（宿需手动执行或告知宿运行）：
```powershell
# 在 Windows 侧 PowerShell 中运行
$syncDir = "C:\Users\22414\dev\third_party\syncthing-rust\sync-loopback\real-net-test\sync\benchmark"
while ($true) {
    Get-ChildItem $syncDir -Filter "benchmark_*.bin" | ForEach-Object {
        $confirmFile = "$($_.FullName).received"
        if (-not (Test-Path $confirmFile)) {
            Set-Content -Path $confirmFile -Value "Received at $(Get-Date)"
        }
    }
    Start-Sleep -Seconds 10
}
```

---

### Task 4：网络抖动 / 断线重连测试

**目标**：验证连接断开后能否自动恢复，Index 和 Block 传输能否续传。

**手动操作步骤**：

```bash
# 1. 确认当前连接
ss -tn | grep 100.73.228.59

# 2. 模拟断线：杀死 syncthing 进程（格雷侧）
pkill -f syncthing
sleep 5

# 3. 在 sync 目录放入新文件
echo "Post-reconnect test $(date)" > ~/syncthing-test/sync/reconnect_test.txt

# 4. 重新启动 syncthing
./syncthing run --config-dir ~/syncthing-test &

# 5. 观察 Windows 侧是否在 30 秒内重新建立连接并同步文件
# 本侧检查：
ss -tn | grep 100.73.228.59
ls ~/syncthing-test/sync/  # 检查 Windows 传来的文件是否仍在
```

**自动化脚本**（需 root 权限修改 iptables）：
```bash
#!/bin/bash
# network_chaos.sh — 网络抖动测试

for i in $(seq 1 10); do
    echo "[$(date)] Round $i: Blocking port 22001 for 10s..."
    sudo iptables -A INPUT -p tcp --dport 22001 -j DROP
    sleep 10

    echo "[$(date)] Round $i: Restoring port 22001..."
    sudo iptables -D INPUT -p tcp --dport 22001 -j DROP
    sleep 30

    # 检查连接恢复
    if ss -tn | grep -q "100.73.228.59.*ESTAB"; then
        echo "[$(date)] Round $i: Connection RESTORED ✅"
    else
        echo "[$(date)] Round $i: Connection NOT restored ❌"
    fi
done
```

---

### Task 5：日志收集与监控

**持续收集格雷侧日志**：

```bash
# 创建日志收集脚本
cat > ~/syncthing-test/collect_logs.sh <> 'IGNOREEOF'
#!/bin/bash
LOG_DIR="$HOME/syncthing-test/logs"
mkdir -p "$LOG_DIR"
PID=$(pgrep -f syncthing)
if [ -n "$PID" ]; then
    echo "$(date): syncthing PID=$PID RSS=$(ps -o rss= -p $PID)KB Connections=$(ss -tn | grep 100.73.228.59 | wc -l)" >> "$LOG_DIR/metrics.log"
fi
IGNOREEOF
chmod +x ~/syncthing-test/collect_logs.sh

# 加入 crontab，每 5 分钟执行一次
(crontab -l 2>/dev/null; echo "*/5 * * * * $HOME/syncthing-test/collect_logs.sh") | crontab -
```

**关键日志关键词**（格雷排查时 grep 用）：
```bash
# 连接状态
grep -i "established\|disconnect\|handshake" ~/syncthing-test/logs/*.log

# 同步事件
grep -i "indexupdate\|block.request\|block.response\|download.completed" ~/syncthing-test/logs/*.log

# 错误
grep -i "error\|panic\|nosuchfile\|failed" ~/syncthing-test/logs/*.log
```

---

## 5. REST API 速查（供自动化脚本调用）

```bash
API_KEY=$(grep -o '"api_key": "[^"]*"' ~/syncthing-test/config.json | cut -d'"' -f4)
BASE_URL="http://127.0.0.1:8385"

# 查询系统状态
curl -s -H "X-API-Key: $API_KEY" "$BASE_URL/rest/system/status" | jq .

# 查询连接状态
curl -s -H "X-API-Key: $API_KEY" "$BASE_URL/rest/system/connections" | jq .

# 查询文件夹状态
curl -s -H "X-API-Key: $API_KEY" "$BASE_URL/rest/db/status?folder=test-folder" | jq .

# 查询文件夹文件列表
curl -s -H "X-API-Key: $API_KEY" "$BASE_URL/rest/db/browse?folder=test-folder" | jq .
```

⚠️ **注意**：早期版本（≤ v0.2.7）部分 API 端点可能存在空引用异常；当前 v3.0.x 已修复。若仍返回 500，请使用 `netstat` 和文件系统检查代替。

---

## 6. 紧急联系与上报

| 情况 | 操作 |
|------|------|
| 守护进程崩溃 / panic | 保存 `stderr.log`、`stdout.log`，上报宿 |
| 连接无法建立（SYN_SENT 挂死） | 检查 Tailscale 状态 (`tailscale status`)，检查本侧防火墙 |
| 文件同步中断（Index 停止更新） | 重启双方守护进程，检查 `.stignore` 是否误过滤 |
| 内存泄漏（RSS > 500MB） | 收集 `metrics.log`，立即上报宿 |

---

**祝任务顺利。宿与格雷并肩作战。**
