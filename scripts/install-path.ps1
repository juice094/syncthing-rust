# syncthing-rust PATH 安装脚本
# 将 Rust 版 syncthing 目录添加到用户 PATH，优先级高于 Go 版.
# 运行方式: powershell -ExecutionPolicy Bypass -File install-path.ps1

$ErrorActionPreference = "Stop"

$releaseDir = "C:\Users\22414\dev\syncthing-rust\target\release"
if (-not (Test-Path "$releaseDir\syncthing.exe")) {
    Write-Host "ERROR: syncthing.exe not found at $releaseDir" -ForegroundColor Red
    Write-Host "请先运行: cargo build --release --bin syncthing" -ForegroundColor Yellow
    exit 1
}

# 读取当前用户 PATH
$currentUserPath = [Environment]::GetEnvironmentVariable("Path", "User") ?? ""

if ($currentUserPath -split ";" | Where-Object { $_ -eq $releaseDir }) {
    Write-Host "Already in PATH: $releaseDir" -ForegroundColor Green
} else {
    $newPath = "$releaseDir;$currentUserPath"
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Added to user PATH: $releaseDir" -ForegroundColor Green
    Write-Host "请重新打开终端使 PATH 生效" -ForegroundColor Yellow
}

# 检查当前 PATH 中哪个 syncthing.exe 被优先找到
$found = Get-Command syncthing.exe -ErrorAction SilentlyContinue
if ($found) {
    Write-Host "当前 syncthing 路径: $($found.Source)" -ForegroundColor Cyan
}
