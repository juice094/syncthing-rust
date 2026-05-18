#!/usr/bin/env pwsh
# Pull remote metrics CSV from the test node via scp.
#
# Usage: .\pull-remote-metrics.ps1 -RemoteHost "user@100.127.13.26" `
#                                    -RemotePath "/tmp/syncthing-test-node/metrics/monitor.csv" `
#                                    -LocalDir ".\remote-metrics" `
#                                    -IntervalSeconds 300
#
# Designed for the 72h dual-node endurance test.

param(
    [Parameter(Mandatory = $true)]
    [string]$RemoteHost,

    [Parameter(Mandatory = $true)]
    [string]$RemotePath,

    [string]$LocalDir = ".\remote-metrics",

    [int]$IntervalSeconds = 300,

    [string]$LogFile = ""
)

$ErrorActionPreference = "Continue"

if ($LogFile -eq "") {
    $LogFile = Join-Path $LocalDir "pull.log"
}

function Write-Log {
    param([string]$Message)
    $ts = (Get-Date -Format "yyyy-MM-ddTHH:mm:ssZ")
    $line = "[$ts] $Message"
    Write-Host $line
    try {
        $line | Out-File -FilePath $LogFile -Append -Encoding utf8 -ErrorAction SilentlyContinue
    } catch {}
}

# Ensure local directory exists
if (-not (Test-Path $LocalDir)) {
    New-Item -ItemType Directory -Path $LocalDir -Force | Out-Null
}

# Derive a local filename with timestamp to avoid overwriting
$remoteBaseName = [System.IO.Path]::GetFileName($RemotePath)

Write-Log "Starting remote metrics pull from ${RemoteHost}:${RemotePath}"
Write-Log "Local dir: $LocalDir  Interval: ${IntervalSeconds}s"

$consecutiveErrors = 0
$maxConsecutiveErrors = 12  # ~1 hour of failures before giving up

while ($true) {
    $ts = Get-Date -Format "yyyyMMdd_HHmmss"
    $localFile = Join-Path $LocalDir "${remoteBaseName}.${ts}"

    try {
        # Use scp to pull the file. -O forces legacy scp protocol (more compatible).
        $scpArgs = @("-O", "${RemoteHost}:${RemotePath}", $localFile)
        $proc = Start-Process -FilePath "scp" -ArgumentList $scpArgs -Wait -PassThru -NoNewWindow `
            -RedirectStandardError (Join-Path $env:TEMP "scp-err-${ts}.txt")

        if ($proc.ExitCode -eq 0) {
            $size = (Get-Item $localFile -ErrorAction SilentlyContinue).Length
            Write-Log "Pulled ${remoteBaseName} -> ${localFile} (${size} bytes)"
            $consecutiveErrors = 0
        } else {
            $err = Get-Content (Join-Path $env:TEMP "scp-err-${ts}.txt") -ErrorAction SilentlyContinue
            Write-Log "scp failed (exit $($proc.ExitCode)): $err"
            $consecutiveErrors++
        }
    } catch {
        Write-Log "Exception during scp: $_"
        $consecutiveErrors++
    }

    if ($consecutiveErrors -ge $maxConsecutiveErrors) {
        Write-Log "Too many consecutive errors ($consecutiveErrors), stopping."
        break
    }

    Start-Sleep -Seconds $IntervalSeconds
}

Write-Log "Remote metrics pull stopped."
