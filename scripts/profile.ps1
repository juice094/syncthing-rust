# syncthing-rust 一键 profile 脚本（T-A2）
# 制定日期: 2026-05-11
# 用法:
#   .\scripts\profile.ps1 -Mode cpu      -Duration 5m
#   .\scripts\profile.ps1 -Mode heap     -Duration 1h
#   .\scripts\profile.ps1 -Mode tasks
#   .\scripts\profile.ps1 -Mode bench    -Crate bep-protocol

param(
    [Parameter(Mandatory=$true)]
    [ValidateSet('cpu','heap','tasks','bench')]
    [string]$Mode,

    [string]$Duration = '5m',
    [string]$Crate = '',
    [string]$OutDir = 'reports/profiling'
)

$ErrorActionPreference = 'Stop'

# 确保输出目录存在
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'

function Write-Section($title) {
    Write-Host ""
    Write-Host "===== $title =====" -ForegroundColor Cyan
}

function Test-Tool($name, $installHint) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
        Write-Host "❌ 缺少工具：$name" -ForegroundColor Red
        Write-Host "   安装：$installHint" -ForegroundColor Yellow
        exit 1
    }
}

switch ($Mode) {
    'cpu' {
        Write-Section "CPU 火焰图采集（持续 $Duration）"
        Test-Tool 'cargo' 'rustup update'
        Test-Tool 'cargo-flamegraph' 'cargo install flamegraph'

        $env:CARGO_PROFILE_RELEASE_DEBUG = 'true'
        $svgPath = "$OutDir/flamegraph-$timestamp.svg"

        Write-Host "采集中... 输出 → $svgPath"
        cargo flamegraph --output $svgPath --bin stress_test --release `
            -- --duration $Duration --report "$OutDir/stress-$timestamp.csv"

        Write-Host "✅ 完成。浏览器打开 $svgPath" -ForegroundColor Green
    }

    'heap' {
        Write-Section "堆分配热点采集（dhat，持续 $Duration）"
        Write-Host "前置：必须在 cmd/syncthing/Cargo.toml 加 [features] dhat-heap = ['dep:dhat']" -ForegroundColor Yellow

        $jsonPath = "$OutDir/dhat-heap-$timestamp.json"
        $env:DHAT_OUT = $jsonPath

        cargo run --release --features dhat-heap --bin stress_test `
            -- --duration $Duration --report "$OutDir/stress-$timestamp.csv"

        Write-Host "✅ 完成。上传 $jsonPath 到 https://nnethercote.github.io/dh_view/dh_view.html" -ForegroundColor Green
    }

    'tasks' {
        Write-Section "tokio-console 实时 task 可视化"
        Test-Tool 'tokio-console' 'cargo install tokio-console'

        Write-Host "前置：必须 console-subscriber + RUSTFLAGS=--cfg tokio_unstable" -ForegroundColor Yellow
        Write-Host ""
        Write-Host "Terminal 1（启动 daemon）：" -ForegroundColor Yellow
        Write-Host "  `$env:RUSTFLAGS = '--cfg tokio_unstable'"
        Write-Host "  cargo run --release --bin syncthing -- run"
        Write-Host ""
        Write-Host "Terminal 2（本窗口将自动启动 tokio-console）：" -ForegroundColor Yellow
        tokio-console
    }

    'bench' {
        Write-Section "criterion 基准（crate: $Crate）"
        if ([string]::IsNullOrEmpty($Crate)) {
            Write-Host "请指定 -Crate <crate-name>，如 bep-protocol / syncthing-fs / syncthing-sync" -ForegroundColor Red
            exit 1
        }

        $reportDir = "$OutDir/criterion-$timestamp"
        New-Item -ItemType Directory -Force -Path $reportDir | Out-Null

        cargo bench -p $Crate 2>&1 | Tee-Object -FilePath "$reportDir/bench.log"

        # 复制 criterion HTML 报告
        if (Test-Path "target/criterion") {
            Copy-Item -Recurse -Force "target/criterion" "$reportDir/"
            Write-Host "✅ 完成。打开 $reportDir/criterion/report/index.html" -ForegroundColor Green
        } else {
            Write-Host "⚠️ target/criterion 不存在；可能该 crate 尚未配置 bench" -ForegroundColor Yellow
        }
    }
}

Write-Section "Done · 报告目录"
Get-ChildItem $OutDir -Filter "*-$timestamp*" | Select-Object Name, Length | Format-Table -AutoSize
