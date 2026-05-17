#Requires -Version 5.1
<#
.SYNOPSIS
    Stop the two-node real-network stress test processes.

.DESCRIPTION
    Reads status.json and gracefully terminates daemon, monitor, and churn.

.PARAMETER DataDir
    Root directory for test data (must match the one used when starting).
#
>
param(
    [string]$DataDir = "$env:USERPROFILE\syncthing-two-node-test"
)

$ErrorActionPreference = "Continue"

$statusPath = "$DataDir\status.json"
if (-not (Test-Path $statusPath)) {
    Write-Warning "status.json not found at $statusPath. Trying to find processes by name..."

    $procs = Get-Process -Name "syncthing" -ErrorAction SilentlyContinue
    foreach ($p in $procs) {
        Write-Host "Stopping syncthing PID $($p.Id)"
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }

    $monitors = Get-Process -Name "syncthing-monitor" -ErrorAction SilentlyContinue
    foreach ($p in $monitors) {
        Write-Host "Stopping syncthing-monitor PID $($p.Id)"
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }

    $jobs = Get-Job -Name "churn*" -ErrorAction SilentlyContinue
    foreach ($j in $jobs) {
        Write-Host "Stopping churn job $($j.Id)"
        Stop-Job -Id $j.Id -ErrorAction SilentlyContinue
        Remove-Job -Id $j.Id -ErrorAction SilentlyContinue
    }

    Write-Host "Done."
    return
}

$status = Get-Content $statusPath -Raw | ConvertFrom-Json

Write-Host "=== Stopping Two-Node Test ===" -ForegroundColor Cyan

if ($status.local_pid) {
    Write-Host "Stopping daemon (PID: $($status.local_pid)) ..."
    Stop-Process -Id $status.local_pid -Force -ErrorAction SilentlyContinue
}

if ($status.monitor_pid) {
    Write-Host "Stopping monitor (PID: $($status.monitor_pid)) ..."
    Stop-Process -Id $status.monitor_pid -Force -ErrorAction SilentlyContinue
}

if ($status.churn_job_id) {
    Write-Host "Stopping churn job (Id: $($status.churn_job_id)) ..."
    Stop-Job -Id $status.churn_job_id -ErrorAction SilentlyContinue
    Remove-Job -Id $status.churn_job_id -ErrorAction SilentlyContinue
}

Write-Host "Done." -ForegroundColor Green
Write-Host ""
Write-Host "Logs and metrics preserved in: $DataDir"
