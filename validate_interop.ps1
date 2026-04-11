#!/usr/bin/env pwsh
# 本地互操作验证脚本
# 用法：先启动 Go 节点（start_go_syncthing.ps1），再启动 Rust 节点（start_rust_syncthing.ps1），然后运行此脚本

$ErrorActionPreference = "Continue"
$apiKey = "syncthing-test-apikey"
$goApi = "http://127.0.0.1:8384/rest"
$goFolder = "C:\Users\22414\dev\third_party\syncthing\test_go_home\test-folder"
$rustFolder = "C:\Users\22414\dev\third_party\syncthing-rust\test_rust_folder"

function Write-Header($msg) { Write-Host "`n=== $msg ===" -ForegroundColor Cyan }
function Write-Ok($msg)    { Write-Host "[OK]   $msg" -ForegroundColor Green }
function Write-Warn($msg)  { Write-Host "[WARN] $msg" -ForegroundColor Yellow }
function Write-Fail($msg)  { Write-Host "[FAIL] $msg" -ForegroundColor Red }

Write-Header "Step 1: Check Go Syncthing status"
try {
    $status = Invoke-RestMethod -Uri "$goApi/system/connections" -Headers @{ "X-API-Key" = $apiKey } -TimeoutSec 5
    Write-Ok "Go Syncthing API is responding"
    $connected = $status.connections.PSObject.Properties | Where-Object { $_.Value.connected -eq $true }
    if ($connected) {
        Write-Ok "At least one peer is connected to Go"
        $connected | ForEach-Object { Write-Host "       - $($_.Name): connected=$($_.Value.connected)" -ForegroundColor Gray }
    } else {
        Write-Warn "No peers currently connected to Go (wait a few seconds and retry)"
    }
} catch {
    Write-Fail "Cannot reach Go Syncthing API: $_"
    Write-Host "Make sure Go node is running: .\start_go_syncthing.ps1" -ForegroundColor Yellow
    exit 1
}

Write-Header "Step 2: Folder contents check"
Write-Host "Go folder:    $goFolder" -ForegroundColor Gray
Write-Host "Rust folder:  $rustFolder" -ForegroundColor Gray

$goFiles = if (Test-Path $goFolder) { Get-ChildItem -File -Recurse $goFolder | Select-Object -ExpandProperty FullName } else { @() }
$rustFiles = if (Test-Path $rustFolder) { Get-ChildItem -File -Recurse $rustFolder | Select-Object -ExpandProperty FullName } else { @() }

Write-Host "Go files:   $($goFiles.Count)" -ForegroundColor Gray
Write-Host "Rust files: $($rustFiles.Count)" -ForegroundColor Gray

$goFiles | ForEach-Object { Write-Host "  [GO]   $_" -ForegroundColor Gray }
$rustFiles | ForEach-Object { Write-Host "  [RUST] $_" -ForegroundColor Gray }

Write-Header "Step 3: Manual test instructions"
Write-Host @"
To verify end-to-end sync:

1. Drop a new file into the RUST folder:
   echo "from rust" > "$rustFolder\rust_test.txt"

2. Wait 10-20 seconds, then check the GO folder:
   dir "$goFolder"

3. Conversely, drop a file into the GO folder:
   echo "from go" > "$goFolder\go_test.txt"

4. Wait 10-20 seconds, then check the RUST folder:
   dir "$rustFolder"

If files appear on both sides, the BEP protocol and sync loop are working.
"@
