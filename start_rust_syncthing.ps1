#!/usr/bin/env pwsh
# 启动本地 Rust Syncthing（用于与 Go 节点互操作测试）
$ErrorActionPreference = "Stop"

$rustExe = "C:\Users\22414\dev\third_party\syncthing-rust\target\release\syncthing.exe"
$configDir = "$env:TEMP\syncthing_rust_win"

if (-not (Test-Path $rustExe)) {
    Write-Host "[ERROR] Rust Syncthing executable not found at $rustExe" -ForegroundColor Red
    Write-Host "Run: cargo build --release -p syncthing" -ForegroundColor Yellow
    exit 1
}

Write-Host "Starting Rust Syncthing..." -ForegroundColor Cyan
Write-Host "  Config: $configDir" -ForegroundColor Gray
Write-Host "  Listen: 127.0.0.1:22000" -ForegroundColor Gray
Write-Host "  Test folder: C:\Users\22414\dev\third_party\syncthing-rust\test_rust_folder" -ForegroundColor Gray

# 确保证书存在
if (-not (Test-Path "$configDir\cert.pem")) {
    Write-Host "Generating certificate..." -ForegroundColor Yellow
    & $rustExe --config-dir $configDir generate-cert | Out-Null
}

# 启动
& $rustExe run --config-dir $configDir --listen 127.0.0.1:22000 --device-name rust-syncthing
