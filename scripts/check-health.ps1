#!/usr/bin/env pwsh
# 本地代码健康度检查脚本
# 运行: .\scripts\check-health.ps1

param(
    [switch]$Strict = $false
)

$exitCode = 0

function Write-Step {
    param([string]$Number, [string]$Title)
    Write-Host "`n[$Number/4] $Title" -ForegroundColor Cyan
}

function Write-Result {
    param([string]$Status, [switch]$IsError)
    if ($IsError) {
        Write-Host "  → $Status" -ForegroundColor Red
        $script:exitCode = 1
    } else {
        Write-Host "  → $Status" -ForegroundColor Green
    }
}

Write-Host "=== syncthing-rust Health Check ===" -ForegroundColor White

# 1. 测试
Write-Step -Number "1" -Title "Running cargo test --workspace"
cargo test --workspace 2>&1 | Tee-Object -Variable testOutput | Out-Null
$testResults = $testOutput | Select-String "test result"
$testResults | ForEach-Object { Write-Host "  $_" }
$hasFailed = $testResults | Select-String "[1-9]\d* failed"
if ($hasFailed -or $LASTEXITCODE -ne 0) {
    Write-Result -Status "TESTS FAILED" -IsError
} else {
    Write-Result -Status "All tests passed"
}

# 2. Clippy
Write-Step -Number "2" -Title "Running cargo clippy --all-targets"
cargo clippy --all-targets 2>&1 | Tee-Object -Variable clippyOutput | Out-Null
# Filter to actual warnings (not progress notes)
$warnings = $clippyOutput | Where-Object { $_ -is [string] -and $_ -match "^warning:" }
if ($warnings -or ($LASTEXITCODE -ne 0 -and -not ($clippyOutput -match "Finished"))) {
    Write-Result -Status "$($warnings.Count) clippy warning(s)" -IsError
    $warnings | Select-Object -First 5 | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkYellow }
} else {
    Write-Result -Status "0 warnings"
}

# 3. Audit
Write-Step -Number "3" -Title "Running cargo audit"
$auditJson = cargo audit --json --color never 2>$null
if ($auditJson) {
    try {
        $auditObj = $auditJson | ConvertFrom-Json
        if ($auditObj.vulnerabilities.found -eq $true) {
            Write-Result -Status "Vulnerabilities found" -IsError
        } elseif ($auditObj.warnings.PSObject.Properties.Count -gt 0) {
            Write-Result -Status "Warnings found" -IsError
        } else {
            Write-Result -Status "Clean"
        }
    } catch {
        Write-Result -Status "Audit completed (parse fallback)"
    }
} else {
    Write-Result -Status "Audit output unavailable"
}

# 4. File size check
Write-Step -Number "4" -Title "Checking file size soft limit (600 lines)"
$oversized = Get-ChildItem -Recurse -Filter "*.rs" -Path crates, cmd | ForEach-Object {
    $lines = (Get-Content $_.FullName | Measure-Object -Line).Lines
    if ($lines -gt 600) {
        [PSCustomObject]@{ Lines = $lines; File = $_.FullName.Replace((Get-Location).Path + '\', '') }
    }
}
if ($oversized) {
    Write-Result -Status "$($oversized.Count) file(s) >600 lines" -IsError:$Strict
    $oversized | Sort-Object Lines -Descending | Select-Object -First 10 | Format-Table -AutoSize | Out-String | Write-Host -ForegroundColor DarkYellow
} else {
    Write-Result -Status "All files within limit"
}

Write-Host "`n=== Health Check Complete ===" -ForegroundColor White
exit $exitCode
