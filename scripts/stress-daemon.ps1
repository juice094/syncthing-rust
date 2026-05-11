#requires -version 5.1
<#
.SYNOPSIS
    syncthing-rust 72h 压测守护脚本（Windows 重启自动维系）
.DESCRIPTION
    检查 PID 文件判断 stress_test 是否存活；若未运行则以 --resume 重新启动。
    由 Windows 计划任务每 5 分钟调用一次。
#>
param(
    [string]$RepoRoot = "C:\Users\22414\dev\third_party\syncthing-rust",
    [string]$DataDir  = "stress-test-data",
    [string]$Report   = "stress-test-report.csv",
    [string]$PidFile  = "stress-test.pid"
)

$ErrorActionPreference = "Stop"
$bin = Join-Path $RepoRoot "target\release\stress_test.exe"
$pidPath = Join-Path $RepoRoot $PidFile

function Test-ProcessAlive([int]$pid) {
    try {
        $proc = Get-Process -Id $pid -ErrorAction Stop
        return $proc.ProcessName -match "stress"
    } catch {
        return $false
    }
}

# T-F1: 全局多实例防护 — 扫描并清理所有旧的 stress_test.exe
$existing = Get-Process -Name "stress_test" -ErrorAction SilentlyContinue
if ($existing) {
    foreach ($proc in $existing) {
        Write-Host "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] Killing stale stress_test PID $($proc.Id)"
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 2
}

$needsStart = $true
if (Test-Path $pidPath) {
    $pidStr = Get-Content $pidPath -Raw
    if ([int]::TryParse($pidStr, [ref]$null)) {
        $pidInt = [int]$pidStr.Trim()
        if (Test-ProcessAlive $pidInt) {
            Write-Host "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] PID $pidInt alive, skip"
            $needsStart = $false
        } else {
            Write-Host "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] PID $pidInt dead, will resume"
        }
    }
}

if ($needsStart) {
    $args = @(
        "--duration", "72h",
        "--report",   (Join-Path $RepoRoot $Report),
        "--data-dir", (Join-Path $RepoRoot $DataDir),
        "--pid-file", $pidPath,
        "--inject-interval", "5m",
        "--fault-interval",  "30m",
        "--resume"
    )
    Write-Host "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] Starting stress_test with --resume"
    Start-Process -FilePath $bin -ArgumentList $args -WorkingDirectory $RepoRoot -WindowStyle Hidden
    Start-Sleep -Seconds 3
    if (Test-Path $pidPath) {
        $newPid = [int](Get-Content $pidPath -Raw).Trim()
        Write-Host "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] Started PID $newPid"
    }
}
