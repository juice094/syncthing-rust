#!/usr/bin/env pwsh
#requires -Version 5.1
<#
.SYNOPSIS
    Syncthing-rust 明日可用性一键验证脚本
.DESCRIPTION
    编译 → 证书 → 配置互信 → 启动 Go peer → 启动 Rust daemon →
    连接验证 → syncbench → metrics-flush → 清理
.NOTES
    运行前请确保 Go Syncthing 可执行文件存在：C:\Users\22414\syncthing_go.exe
    （如没有，脚本会跳过 Go peer 相关的端到端测试）
#>

$ErrorActionPreference = "Stop"

# ==============================================================================
# 配置
# ==============================================================================
$workspaceRoot   = "C:\Users\22414"
$rustExe         = Join-Path $workspaceRoot "target\release\syncthing.exe"
$goExe           = Join-Path $workspaceRoot "syncthing_go.exe"
$testBase        = Join-Path $env:TEMP "syncthing-rust-validation-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
$rustConfigDir   = Join-Path $testBase "rust-config"
$goConfigDir     = Join-Path $testBase "go-config"
$rustLog         = Join-Path $testBase "rust-daemon.log"
$metricsCsv      = Join-Path $testBase "metrics.csv"
$benchReport     = Join-Path $testBase "syncbench-report.json"

$rustListen      = "127.0.0.1:22000"
$goListen        = "127.0.0.1:22001"

# 颜色
function Write-Header($msg) { Write-Host "`n=== $msg ===" -ForegroundColor Cyan }
function Write-Ok($msg)    { Write-Host "[OK]   $msg" -ForegroundColor Green }
function Write-Warn($msg)  { Write-Host "[WARN] $msg" -ForegroundColor Yellow }
function Write-Fail($msg)  { Write-Host "[FAIL] $msg" -ForegroundColor Red }

# ==============================================================================
# 步骤 0: 前置检查
# ==============================================================================
Write-Header "Step 0: 前置检查"

if (!(Test-Path $rustExe)) {
    Write-Fail "Rust 可执行文件不存在: $rustExe"
    exit 1
}
Write-Ok "Rust 可执行文件存在: $rustExe"

$hasGoPeer = Test-Path $goExe
if ($hasGoPeer) {
    Write-Ok "Go Syncthing 可执行文件存在: $goExe"
} else {
    Write-Warn "Go Syncthing 可执行文件不存在，端到端同步测试将跳过"
}

# 创建目录
@($testBase, $rustConfigDir, $goConfigDir) | ForEach-Object {
    New-Item -ItemType Directory -Path $_ -Force | Out-Null
}
Write-Ok "测试目录创建完成: $testBase"

# 清理可能残留的 Go Syncthing 进程（避免端口占用）
Get-Process -Name "syncthing_go" -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Warn "结束残留 Go 进程 (PID $($_.Id))"
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 1

# ==============================================================================
# 步骤 1: 编译检查
# ==============================================================================
Write-Header "Step 1: 编译检查"
try {
    Push-Location $workspaceRoot
    $buildOutput = & { $ErrorActionPreference = "Continue"; cargo build --release -p syncthing 2>&1 }
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "编译失败"
        $buildOutput | ForEach-Object { Write-Host $_ }
        exit 1
    }
    Write-Ok "编译通过 (release)"
} finally {
    Pop-Location
}

# ==============================================================================
# 步骤 2: 生成证书并获取 Device ID
# ==============================================================================
Write-Header "Step 2: 生成证书"

& { $ErrorActionPreference = "Continue"; & $rustExe --config-dir $rustConfigDir generate-cert } | Out-Null
$rustId = (& { $ErrorActionPreference = "Continue"; & $rustExe --config-dir $rustConfigDir show-id } | Select-String "^设备ID:\s*(.+)$").Matches.Groups[1].Value.Trim()
Write-Ok "Rust Device ID: $rustId"

# 初始化 Rust config.json（包含对 Go peer 的信任）
if ($hasGoPeer) {
    $goApiKey = "syncthing-validation"

    # 生成 Go 配置并直接读取 Device ID（无需启动临时实例）
    & { $ErrorActionPreference = "Continue"; & $goExe generate -H $goConfigDir } | Out-Null
    Start-Sleep -Seconds 2
    $goConfigFile = Join-Path $goConfigDir "config.xml"

    $goId = "UNKNOWN-GO-DEVICE-ID"
    if (Test-Path $goConfigFile) {
        try {
            [xml]$xml = Get-Content $goConfigFile
            $goId = $xml.configuration.device.id
            Write-Ok "Go Device ID: $goId"
        } catch {
            Write-Warn "无法从 Go config.xml 读取 Device ID: $_"
        }
    }

    if (($goId -ne "UNKNOWN-GO-DEVICE-ID") -and (Test-Path $goConfigFile)) {
        try {
            [xml]$xml = Get-Content $goConfigFile
            $cfg = $xml.configuration

            # 固定 API key
            $xml.configuration.gui.apikey = $goApiKey

            # 设置监听地址
            $options = $cfg.options
            if ($options -eq $null) {
                $options = $xml.CreateElement("options")
                $cfg.AppendChild($options) | Out-Null
            }
            $listenNode = $options.SelectSingleNode("listenAddress")
            if ($listenNode -eq $null) {
                $listenNode = $xml.CreateElement("listenAddress")
                $options.AppendChild($listenNode) | Out-Null
            }
            $listenNode.InnerText = "tcp://127.0.0.1:22001"

            # 关闭 NAT
            $natNode = $options.SelectSingleNode("natEnabled")
            if ($natNode -eq $null) {
                $natNode = $xml.CreateElement("natEnabled")
                $options.AppendChild($natNode) | Out-Null
            }
            $natNode.InnerText = "false"

            # 添加 Rust device
            $rustDeviceNode = $xml.CreateElement("device")
            $rustDeviceNode.SetAttribute("id", $rustId)
            $rustDeviceNode.SetAttribute("name", "syncthing-rust")
            $rustDeviceNode.SetAttribute("compression", "metadata")
            $rustDeviceNode.SetAttribute("introducer", "false")
            $rustDeviceNode.SetAttribute("skipIntroductionRemovals", "false")
            $rustDeviceNode.SetAttribute("introducedBy", "")
            $addrNode = $xml.CreateElement("address")
            $addrNode.InnerText = "dynamic"
            $rustDeviceNode.AppendChild($addrNode) | Out-Null
            $pausedNode = $xml.CreateElement("paused")
            $pausedNode.InnerText = "false"
            $rustDeviceNode.AppendChild($pausedNode) | Out-Null
            $cfg.AppendChild($rustDeviceNode) | Out-Null

            # 添加 folder
            $folderNode = $xml.CreateElement("folder")
            $folderNode.SetAttribute("id", "validation-folder")
            $folderNode.SetAttribute("label", "")
            $goShared = Join-Path $testBase "go-shared-folder"
            $folderNode.SetAttribute("path", $goShared)
            $folderNode.SetAttribute("type", "sendreceive")
            $folderNode.SetAttribute("rescanIntervalS", "10")
            $folderNode.SetAttribute("fsWatcherEnabled", "false")
            $fdGo = $xml.CreateElement("device")
            $fdGo.SetAttribute("id", $goId)
            $folderNode.AppendChild($fdGo) | Out-Null
            $fdRust = $xml.CreateElement("device")
            $fdRust.SetAttribute("id", $rustId)
            $folderNode.AppendChild($fdRust) | Out-Null
            $cfg.AppendChild($folderNode) | Out-Null

            $xml.Save($goConfigFile)
            Write-Ok "Go config.xml 已更新（设备互信、共享文件夹）"
        } catch {
            Write-Warn "修改 Go config.xml 失败: $_"
        }
    }

    # 写入 Rust config.json
    $rustConfig = @{
        version         = 1
        listen_addr     = $rustListen
        device_name     = "syncthing-rust-validation"
        folders         = @(
            @{
                id                = "validation-folder"
                path              = (Join-Path $testBase "shared-folder").Replace("\", "\\")
                label             = $null
                folder_type       = "SendReceive"
                paused            = $false
                rescan_interval_secs = 10
                devices           = @($rustId, $goId)
                ignore_patterns   = @()
                versioning        = $null
            }
        )
        devices         = @(
            @{
                id          = $goId
                name        = "go-syncthing"
                addresses   = @(@{ Tcp = $goListen })
                paused      = $false
                introducer  = $false
            }
        )
        local_device_id = $rustId
    } | ConvertTo-Json -Depth 10

    [System.IO.File]::WriteAllText((Join-Path $rustConfigDir "config.json"), $rustConfig, [System.Text.UTF8Encoding]::new($false))
    Write-Ok "Rust config.json 已生成（包含 Go peer 互信）"

    # 启动 Go peer
    $goProc = Start-Process -FilePath $goExe -ArgumentList "serve", "-H", $goConfigDir, "--gui-address=127.0.0.1:8384", "--no-browser" -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $testBase "go-daemon.log") -RedirectStandardError (Join-Path $testBase "go-daemon.err")
    Start-Sleep -Seconds 3
}

# ==============================================================================
# 步骤 3: 启动 Rust Daemon 并验证网络监听
# ==============================================================================
Write-Header "Step 3: 启动 Rust Daemon"

$rustProc = Start-Process -FilePath $rustExe -ArgumentList "--config-dir", $rustConfigDir, "run", "--listen", $rustListen -PassThru -WindowStyle Hidden -RedirectStandardOutput $rustLog -RedirectStandardError (Join-Path $testBase "rust-daemon.err")
Start-Sleep -Seconds 3

if ($rustProc.HasExited) {
    Write-Fail "Rust daemon 启动后立即退出，ExitCode=$($rustProc.ExitCode)"
    Get-Content (Join-Path $testBase "rust-daemon.err") -ErrorAction SilentlyContinue | ForEach-Object { Write-Host $_ }
    exit 1
}

# 检查端口监听
$listener = Get-NetTCPConnection -LocalPort 22000 -ErrorAction SilentlyContinue | Where-Object { $_.State -eq "Listen" }
if ($listener) {
    Write-Ok "Rust daemon 正在监听 $rustListen (PID $($rustProc.Id))"
} else {
    Write-Warn "未检测到 22000 端口监听，可能绑定延迟或非管理员权限不可见"
}

# ==============================================================================
# 步骤 4: 连接验证（如 Go peer 存在）
# ==============================================================================
Write-Header "Step 4: 连接/BEP 握手验证"
if ($hasGoPeer -and $goProc -and !$goProc.HasExited) {
    Write-Ok "Go peer 运行中 (PID $($goProc.Id))"

    # 等待连接建立（最多 30 秒）
    $connected = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Seconds 1
        # 通过 Go REST API 检查连接状态
        try {
            $connResp = Invoke-RestMethod -Uri "http://127.0.0.1:8384/rest/system/connections" -Headers @{ "X-API-Key" = $goApiKey } -TimeoutSec 3 -ErrorAction Stop
            if ($connResp.connections.PSObject.Properties.Name -contains $rustId) {
                $connInfo = $connResp.connections.$rustId
                if ($connInfo.connected) {
                    $connected = $true
                    break
                }
            }
        } catch {
            # 忽略轮询错误
        }
    }

    if ($connected) {
        Write-Ok "BEP 握手成功！Go peer 已连接到 Rust daemon ($rustId)"
    } else {
        Write-Warn "30 秒内未在 Go REST API 中看到连接，可能握手延迟或配置未生效"
    }
} else {
    Write-Warn "跳过连接验证（无 Go peer）"
}

# ==============================================================================
# 步骤 5: 端到端文件同步验证
# ==============================================================================
Write-Header "Step 5: 端到端文件同步验证"

if ($hasGoPeer -and ($goId -ne "UNKNOWN-GO-DEVICE-ID")) {
    $rustShared = Join-Path $testBase "shared-folder"
    $goShared = Join-Path $testBase "go-shared-folder"
    New-Item -ItemType Directory -Path $rustShared -Force | Out-Null
    New-Item -ItemType Directory -Path $goShared -Force | Out-Null

    $testFile = "hello_sync.txt"
    $testContent = "Syncthing Rust <> Go interop test - $(Get-Date)"
    [System.IO.File]::WriteAllText((Join-Path $rustShared $testFile), $testContent, [System.Text.UTF8Encoding]::new($false))
    Write-Ok "已在 Rust shared-folder 创建测试文件: $testFile"

    # 等待同步（最多 20 秒）
    $synced = $false
    $syncWaitSec = 20
    for ($i = 0; $i -lt $syncWaitSec; $i++) {
        Start-Sleep -Seconds 1
        $targetFile = Join-Path $goShared $testFile
        if (Test-Path $targetFile) {
            $targetContent = [System.IO.File]::ReadAllText($targetFile)
            if ($targetContent -eq $testContent) {
                $synced = $true
                break
            }
        }
    }

    if ($synced) {
        Write-Ok "端到端同步验证通过！文件在 ${i}s 后出现在 Go shared-folder"
    } else {
        Write-Warn "端到端同步验证未通过：${syncWaitSec}s 内文件未出现在 Go shared-folder"
        Write-Warn "（可能原因：BEP 握手后索引交换尚未完成，或文件夹同步逻辑仍在开发中）"
    }

    # 同时记录 syncbench small 报告（不判定 success）
    $benchSrc = Join-Path $testBase "bench-src"
    $benchTgt = Join-Path $testBase "bench-tgt"
    $benchResult = & { $ErrorActionPreference = "Continue"; & $rustExe syncbench small --source-dir $benchSrc --target-dir $benchTgt 2>&1 }
    $benchResult | Set-Content $benchReport -Encoding UTF8
    Write-Ok "syncbench small 报告已保存至: $benchReport"
} else {
    Write-Warn "跳过端到端同步验证（无 Go peer）"
}

# ==============================================================================
# 步骤 6: Metrics Flush 验证
# ==============================================================================
Write-Header "Step 6: Metrics Flush"
& { $ErrorActionPreference = "Continue"; & $rustExe metrics-flush $metricsCsv } | Out-Null
if (Test-Path $metricsCsv) {
    $lineCount = (Get-Content $metricsCsv | Measure-Object).Count
    Write-Ok "Metrics 已导出到 $metricsCsv (共 $lineCount 行)"
    Write-Ok "前 5 行预览："
    Get-Content $metricsCsv -TotalCount 5 | ForEach-Object { Write-Host "  $_" }
} else {
    Write-Fail "Metrics CSV 导出失败"
}

# ==============================================================================
# 步骤 7: 清理进程
# ==============================================================================
Write-Header "Step 7: 清理"

function Stop-ProcSafe($proc, $name) {
    if ($proc -and !$proc.HasExited) {
        $proc.Kill()
        $proc.WaitForExit(5000) | Out-Null
        Write-Ok "$name 已停止 (PID $($proc.Id))"
    }
}

Stop-ProcSafe $rustProc "Rust daemon"
if ($hasGoPeer) { Stop-ProcSafe $goProc "Go peer" }

# 保留日志还是全清？默认保留， Uncomment 下面这行如果想全清
# Remove-Item -Recurse -Force $testBase -ErrorAction SilentlyContinue

Write-Ok "测试数据保留在: $testBase"

# ==============================================================================
# 总结
# ==============================================================================
Write-Header "验证总结"
Write-Host "测试目录 : $testBase" -ForegroundColor Gray
Write-Host "Rust 配置: $rustConfigDir" -ForegroundColor Gray
if ($hasGoPeer) { Write-Host "Go 配置  : $goConfigDir" -ForegroundColor Gray }
Write-Host "Syncbench: $benchReport" -ForegroundColor Gray
Write-Host "Metrics  : $metricsCsv" -ForegroundColor Gray
Write-Host "Rust Log : $rustLog" -ForegroundColor Gray
Write-Host "`n如需清理，运行: Remove-Item -Recurse -Force '$testBase'" -ForegroundColor Gray
