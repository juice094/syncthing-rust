#Requires -Version 5.1
#Requires -PSEdition Desktop
<#
.SYNOPSIS
    Two-node real-network stress test orchestrator.

.DESCRIPTION
    Sets up a syncthing-rust sync test between this Windows machine (local)
    and a remote Linux peer over Tailscale.

    Usage:
        .\two-node-real-network-test.ps1 -RemotePeer "100.127.13.26" -Duration "72h"

.PARAMETER RemotePeer
    Tailscale IP of the remote Linux peer.

.PARAMETER LocalPort
    TCP port for local syncthing daemon.

.PARAMETER RemotePort
    TCP port for remote syncthing daemon.

.PARAMETER Duration
    Test duration, e.g. "72h", "5m", "30s".

.PARAMETER DataDir
    Root directory for test data.
#>
param(
    [Parameter(Mandatory=$true)]
    [string]$RemotePeer,

    [int]$LocalPort = 22001,

    [int]$RemotePort = 22001,

    [string]$Duration = "72h",

    [string]$DataDir = "$env:USERPROFILE\syncthing-two-node-test"
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$BinaryDir = "$RepoRoot\target\release"
$SyncthingExe = "$BinaryDir\syncthing.exe"
$CliExe = "$BinaryDir\syncthing-cli.exe"
$MonitorExe = "$BinaryDir\syncthing-monitor.exe"

# Verify binaries exist
if (-not (Test-Path $SyncthingExe)) {
    Write-Error "syncthing.exe not found at $SyncthingExe. Run: cargo build --release --bin syncthing"
}
if (-not (Test-Path $CliExe)) {
    Write-Error "syncthing-cli.exe not found at $CliExe. Run: cargo build --release --bin syncthing-cli"
}
if (-not (Test-Path $MonitorExe)) {
    Write-Error "syncthing-monitor.exe not found at $MonitorExe. Run: cargo build --release --bin syncthing-monitor"
}

# Directories
$LocalNodeDir = "$DataDir\node-local"
$RemoteNodeDir = "$DataDir\node-remote"
$LocalSyncDir = "$DataDir\sync-local"
$RemoteSyncDir = "$DataDir\sync-remote"
$LogDir = "$DataDir\logs"
$MetricsDir = "$DataDir\metrics"

New-Item -ItemType Directory -Force -Path $LocalNodeDir, $RemoteNodeDir, $LocalSyncDir, $RemoteSyncDir, $LogDir, $MetricsDir | Out-Null

Write-Host "=== Two-Node Real-Network Stress Test ===" -ForegroundColor Cyan
Write-Host "Local (Windows):  $env:COMPUTERNAME :$LocalPort"
Write-Host "Remote (Linux):   $RemotePeer :$RemotePort"
Write-Host "Data dir:         $DataDir"
Write-Host "Duration:         $Duration"
Write-Host ""

# ── Generate certificates ──
function Generate-Cert([string]$configDir, [string]$name) {
    Write-Host "Generating certificate for $name ..."
    $certPath = "$configDir\cert.pem"
    if (Test-Path $certPath) {
        Remove-Item $certPath -Force
    }
    $keyPath = "$configDir\key.pem"
    if (Test-Path $keyPath) {
        Remove-Item $keyPath -Force
    }
    & $CliExe --config-dir $configDir generate-cert --force | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Certificate generation failed for $name" }

    $output = & $CliExe --config-dir $configDir show-id 2>&1
    if ($output -match "Device ID:\s+([A-Z0-9-]+)") {
        return $Matches[1]
    }
    throw "Failed to extract device ID for $name"
}

$LocalDeviceId = Generate-Cert $LocalNodeDir "local"
$RemoteDeviceId = Generate-Cert $RemoteNodeDir "remote"

Write-Host "Local Device ID:  $LocalDeviceId"
Write-Host "Remote Device ID: $RemoteDeviceId"
Write-Host ""

# ── Create configs ──
function Create-Config([string]$path, [string]$listen, [string]$deviceName,
                       [string]$peerId, [string]$peerAddr, [string]$syncPath) {
    $config = @{
        version = 1
        listen_addr = $listen
        device_name = $deviceName
        folders = @(
            @{
                id = "stress-test"
                label = "Stress Test Folder"
                path = $syncPath.Replace("\", "/")
                devices = @($peerId)
                rescan_interval_secs = 10
                versioning = @{ type = "" }
            }
        )
        devices = @(
            @{
                id = $peerId
                name = "peer"
                addresses = @("tcp://$peerAddr")
                paused = $false
                introducer = $false
            }
        )
        local_device_id = $null
        gui = @{ enabled = $false; address = "127.0.0.1:8384"; api_key = "" }
        options = @{ relays_enabled = $false }
    } | ConvertTo-Json -Depth 10
    $config | Set-Content -Path $path -Encoding UTF8
}

Create-Config "$LocalNodeDir\config.json" "0.0.0.0:$LocalPort" "node-local" `
    $RemoteDeviceId "$RemotePeer`:$RemotePort" $LocalSyncDir

Create-Config "$RemoteNodeDir\config.json" "0.0.0.0:$RemotePort" "node-remote" `
    $LocalDeviceId "$env:COMPUTERNAME`:$LocalPort" $RemoteSyncDir

Write-Host "Configs created." -ForegroundColor Green
Write-Host ""

# ── Package remote node for Linux ──
$RemoteDeployDir = "$DataDir\deploy-remote"
New-Item -ItemType Directory -Force -Path $RemoteDeployDir | Out-Null

# Copy remote node cert, key, config
Copy-Item "$RemoteNodeDir\cert.pem" "$RemoteDeployDir\"
Copy-Item "$RemoteNodeDir\key.pem" "$RemoteDeployDir\"
Copy-Item "$RemoteNodeDir\config.json" "$RemoteDeployDir\"

# Generate Linux start script
$LinuxStartScript = @"
#!/bin/bash
# Auto-generated Linux start script for two-node stress test
# Generated: $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")
set -euo pipefail

SCRIPT_DIR=\"\$(cd \"\$(dirname "\$0")\" && pwd)\"
DATA_DIR=\"\$SCRIPT_DIR\"
LOG_DIR=\"\$DATA_DIR/logs\"
METRICS_DIR=\"\$DATA_DIR/metrics\"

mkdir -p \"\$LOG_DIR\" \"\$METRICS_DIR\"

echo \"=== Starting Remote Node (Linux) ===\"
echo \"Device ID: $RemoteDeviceId\"
echo \"Peer:      $env:COMPUTERNAME :$LocalPort\"
echo \"Listen:    0.0.0.0:$RemotePort\"
echo \""

# Build syncthing from source if binary not present
SYNCTHING_BIN=\"\$DATA_DIR/syncthing\"
if [[ ! -x \"\$SYNCTHING_BIN\" ]]; then
    echo \"syncthing binary not found. Building from source...\"
    # Clone or use existing repo
    if [[ -d \"/tmp/syncthing-rust\" ]]; then
        cd \"/tmp/syncthing-rust\"
        git pull
    else
        echo \"Please ensure syncthing-rust repo is cloned and run:\"
        echo \"  cargo build --release --bin syncthing\"
        echo \"Then copy target/release/syncthing to this directory.\"
        exit 1
    fi
    cargo build --release --bin syncthing
    cp target/release/syncthing \"\$SYNCTHING_BIN\"
fi

# Start daemon
nohup \"\$SYNCTHING_BIN\" run --config-dir \"\$DATA_DIR\" --listen \"0.0.0.0:$RemotePort\" >> \"\$LOG_DIR/daemon.log\" 2>&1 &
echo \$! > \"\$DATA_DIR/daemon.pid\"
echo \"Daemon started (PID: \$(cat \$DATA_DIR/daemon.pid))\"

# Start monitor if syncthing-monitor is available
MONITOR_BIN=\"\$DATA_DIR/syncthing-monitor\"
if [[ -x \"\$MONITOR_BIN\" ]]; then
    nohup \"\$MONITOR_BIN\" \
        --proc \"\$(cat \$DATA_DIR/daemon.pid)\" \
        --log \"\$LOG_DIR/daemon.log\" \
        --sync-dir \"\$DATA_DIR/sync\" \
        --interval 60s \
        --output \"\$METRICS_DIR/monitor.csv\" \
        --alerts \"\$METRICS_DIR/alerts.log\" \
        >> \"\$LOG_DIR/monitor.log\" 2>&1 &
echo \$! > \"\$DATA_DIR/monitor.pid\"
    echo \"Monitor started\"
fi

echo \""
echo \"To stop: kill \$(cat \$DATA_DIR/daemon.pid)\"
echo \"Logs:    tail -f \$LOG_DIR/daemon.log\"
echo \"Metrics: \$METRICS_DIR/monitor.csv\"
"@

$LinuxStartScript | Set-Content -Path "$RemoteDeployDir\start.sh" -Encoding UTF8

# Also create a README for the remote deployment
$RemoteReadme = @"
# Remote Node Deployment

## Prerequisites
1. Rust toolchain installed (https://rustup.rs)
2. syncthing-rust source code available

## Setup
1. Copy this entire directory to the Linux machine:
   ```
   scp -r deploy-remote/ user@${RemotePeer}:/tmp/syncthing-test-node/
   ```
2. Build the syncthing binary on Linux:
   ```
   cd /path/to/syncthing-rust
   cargo build --release --bin syncthing
   cargo build --release --bin syncthing-monitor
   cp target/release/syncthing /tmp/syncthing-test-node/
   cp target/release/syncthing-monitor /tmp/syncthing-test-node/
   ```
3. Start the remote node:
   ```
   cd /tmp/syncthing-test-node
   bash start.sh
   ```

## Configuration
- Device ID: $RemoteDeviceId
- Listen: 0.0.0.0:$RemotePort
- Peer: $env:COMPUTERNAME ($LocalDeviceId) at $env:COMPUTERNAME:$LocalPort
- Sync folder: ./sync/

## Monitoring
Local metrics: metrics/monitor.csv
Alerts: metrics/alerts.log
"@

$RemoteReadme | Set-Content -Path "$RemoteDeployDir\README.md" -Encoding UTF8

Write-Host "Remote deployment package created: $RemoteDeployDir" -ForegroundColor Green
Write-Host ""

# ── Start local daemon ──
Write-Host "[1/3] Starting local syncthing daemon ..." -ForegroundColor Yellow
$daemonLog = "$LogDir\daemon.log"
$daemonArgs = @("run", "--config-dir", $LocalNodeDir, "--listen", "0.0.0.0:$LocalPort")
$daemonProcess = Start-Process -FilePath $SyncthingExe -ArgumentList $daemonArgs `
    -RedirectStandardOutput $daemonLog -RedirectStandardError $daemonLog `
    -WindowStyle Hidden -PassThru
$daemonProcess.Id | Set-Content -Path "$DataDir\daemon.pid" -NoNewline
Write-Host "Daemon started (PID: $($daemonProcess.Id))"
Write-Host "Log: $daemonLog"
Write-Host ""

# Wait for daemon to initialize and generate cert if needed
Start-Sleep -Seconds 3

# ── Start monitor ──
Write-Host "[2/3] Starting syncthing-monitor ..." -ForegroundColor Yellow
$monitorCsv = "$MetricsDir\monitor.csv"
$monitorLog = "$LogDir\monitor.log"
$monitorArgs = @(
    "--proc", $daemonProcess.Id,
    "--log", $daemonLog,
    "--sync-dir", $LocalSyncDir,
    "--interval", "60s",
    "--output", $monitorCsv,
    "--alerts", "$MetricsDir\alerts.log",
    "--json", "$MetricsDir\monitor.jsonl"
)
$monitorProcess = Start-Process -FilePath $MonitorExe -ArgumentList $monitorArgs `
    -RedirectStandardOutput $monitorLog -RedirectStandardError $monitorLog `
    -WindowStyle Hidden -PassThru
$monitorProcess.Id | Set-Content -Path "$DataDir\monitor.pid" -NoNewline
Write-Host "Monitor started (PID: $($monitorProcess.Id))"
Write-Host "CSV:  $monitorCsv"
Write-Host "JSON: $MetricsDir\monitor.jsonl"
Write-Host ""

# ── Start file churn ──
Write-Host "[3/3] Starting file churn ..." -ForegroundColor Yellow
$churnLog = "$LogDir\churn.log"

# Use a background job for file churn since it's pure PowerShell
$churnJob = Start-Job -ScriptBlock {
    param($syncDir, $logFile, $duration)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $maxDuration = if ($duration -match "^(\d+)([hms])$") {
        $val = [int]$Matches[1]
        switch ($Matches[2]) {
            "s" { [TimeSpan]::FromSeconds($val) }
            "m" { [TimeSpan]::FromMinutes($val) }
            "h" { [TimeSpan]::FromHours($val) }
            default { [TimeSpan]::FromHours(72) }
        }
    } else { [TimeSpan]::FromHours(72) }

    $sizes = @(1024, 64*1024, 1024*1024, 10*1024*1024)
    $counter = 0

    Add-Content -Path $logFile -Value "[churn] Starting in $syncDir, duration=$duration"

    while ($sw.Elapsed -lt $maxDuration) {
        $counter++
        $size = $sizes[($counter - 1) % $sizes.Length]
        $file = Join-Path $syncDir ("file_{0:D4}.dat" -f $counter)
        $data = New-Object byte[] $size
        [System.Random]::new().NextBytes($data)
        [System.IO.File]::WriteAllBytes($file, $data)
        Add-Content -Path $logFile -Value ("[churn] CREATE {0} ({1} bytes)" -f (Split-Path $file -Leaf), $size)

        # Modify older file
        if ($counter -gt 3) {
            $oldFile = Join-Path $syncDir ("file_{0:D4}.dat" -f ($counter - 3))
            if (Test-Path $oldFile) {
                $oldSize = $sizes[($counter - 4) % $sizes.Length]
                $oldData = New-Object byte[] $oldSize
                [System.Random]::new($counter).NextBytes($oldData)
                [System.IO.File]::WriteAllBytes($oldFile, $oldData)
                Add-Content -Path $logFile -Value ("[churn] MODIFY {0}" -f (Split-Path $oldFile -Leaf))
            }
        }

        # Delete oldest
        if ($counter -gt 6) {
            $oldFile = Join-Path $syncDir ("file_{0:D4}.dat" -f ($counter - 6))
            if (Test-Path $oldFile) {
                Remove-Item $oldFile -Force
                Add-Content -Path $logFile -Value ("[churn] DELETE {0}" -f (Split-Path $oldFile -Leaf))
            }
        }

        Start-Sleep -Seconds 30
    }

    Add-Content -Path $logFile -Value "[churn] Duration reached, stopping."
} -ArgumentList $LocalSyncDir, $churnLog, $Duration

Write-Host "Churn job started (Id: $($churnJob.Id))"
Write-Host "Log: $churnLog"
Write-Host ""

# ── Summary ──
Write-Host "=== Test Orchestration Complete ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "Local node:" -ForegroundColor White
Write-Host "  Config:  $LocalNodeDir\config.json"
Write-Host "  Sync:    $LocalSyncDir"
Write-Host "  Log:     $daemonLog"
Write-Host "  PID:     $($daemonProcess.Id)"
Write-Host ""
Write-Host "Remote node deploy package:" -ForegroundColor White
Write-Host "  Path:    $RemoteDeployDir"
Write-Host "  Copy to Linux: scp -r `"$RemoteDeployDir`" user@${RemotePeer}:/tmp/syncthing-test-node/"
Write-Host ""
Write-Host "Monitor:" -ForegroundColor White
Write-Host "  CSV:     $monitorCsv"
Write-Host "  Alerts:  $MetricsDir\alerts.log"
Write-Host "  PID:     $($monitorProcess.Id)"
Write-Host ""
Write-Host "To stop everything:" -ForegroundColor Yellow
Write-Host "  Stop-Process -Id $($daemonProcess.Id)"
Write-Host "  Stop-Process -Id $($monitorProcess.Id)"
Write-Host "  Stop-Job -Id $($churnJob.Id); Remove-Job -Id $($churnJob.Id)"
Write-Host ""
Write-Host "To watch logs:" -ForegroundColor Yellow
Write-Host "  Get-Content '$daemonLog' -Wait -Tail 20"
Write-Host "  Get-Content '$monitorCsv' -Wait -Tail 5"
Write-Host ""

# Save a status file for later reference
$status = @{
    timestamp = (Get-Date -Format "o")
    local_pid = $daemonProcess.Id
    monitor_pid = $monitorProcess.Id
    churn_job_id = $churnJob.Id
    local_device_id = $LocalDeviceId
    remote_device_id = $RemoteDeviceId
    remote_peer = $RemotePeer
    local_port = $LocalPort
    remote_port = $RemotePort
    data_dir = $DataDir
} | ConvertTo-Json

$status | Set-Content -Path "$DataDir\status.json" -Encoding UTF8
