# scripts/agent-facts.ps1
#
# 只读事实导出：从**单一事实源**抽取易漂移的关键数值，供文档 / Agent 引用。
# 存在的理由：本仓曾因「同一事实多处手抄」付出过代价（版本号、测试数、CI job 数）。
# 用法：pwsh scripts/agent-facts.ps1
#
# 不执行任何构建或测试，也不写任何文件。

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Get-FirstMatch {
    param([string]$Path, [string]$Pattern)
    $m = Select-String -Path $Path -Pattern $Pattern | Select-Object -First 1
    if ($null -eq $m) { return '<未找到>' }
    return $m.Matches[0].Groups[1].Value
}

Write-Output "== syncthing-rust 事实导出（只读） =="
Write-Output ("root: {0}" -f $root)

# 版本：根 Cargo.toml 的 [workspace.package] version
Write-Output ("version: {0}" -f (Get-FirstMatch -Path (Join-Path $root 'Cargo.toml') -Pattern '^\s*version\s*=\s*"(\d+\.\d+\.\d+)"'))

# 端口：syncthing-core 常量 + relay-server 子命令默认值
$constants = Join-Path $root 'crates/syncthing-core/src/constants.rs'
$mainRs = Join-Path $root 'cmd/syncthing/src/main.rs'
Write-Output ("bep_port: {0}" -f (Get-FirstMatch -Path $constants -Pattern 'DEFAULT_BEP_PORT:\s*u16\s*=\s*(\d+)'))
Write-Output ("api_port: {0}" -f (Get-FirstMatch -Path $constants -Pattern 'DEFAULT_API_PORT:\s*u16\s*=\s*(\d+)'))
$relayBlock = [regex]::Match((Get-Content $mainRs -Raw), 'RelayServer\s*\{([\s\S]*?)\n\s{4}\}')
if ($relayBlock.Success) {
    $relayDefaults = [regex]::Matches($relayBlock.Groups[1].Value, 'default_value\s*=\s*"([^"]+)"') | ForEach-Object { $_.Groups[1].Value }
    Write-Output ("relay_server_defaults: {0}" -f ($relayDefaults -join ' / '))
} else {
    Write-Output "relay_server_defaults: <未找到 RelayServer 子命令>"
}

# MCP 工具数量：名册中每个工具恰好有一个 inputSchema（资源没有）
$toolsRs = Join-Path $root 'cmd/syncthing-mcp-bridge/src/tools.rs'
$toolCount = (Select-String -Path $toolsRs -Pattern '"inputSchema"' -AllMatches).Matches.Count
Write-Output ("mcp_tools: {0}" -f $toolCount)

# CI job：ci.yml 里 jobs 段下的顶层 key
$ciYml = Join-Path $root '.github/workflows/ci.yml'
$inJobs = $false
$jobs = @()
foreach ($line in Get-Content $ciYml) {
    if ($line -match '^jobs:\s*$') { $inJobs = $true; continue }
    if ($inJobs -and $line -match '^  ([A-Za-z0-9_-]+):\s*$') { $jobs += $Matches[1] }
}
Write-Output ("ci_jobs: {0} ({1})" -f $jobs.Count, ($jobs -join ', '))

# 生产代码超 600 行软上限的文件（与 CI 的 file-size job 同口径）
Write-Output "-- production files > 600 lines --"
Get-ChildItem -Recurse (Join-Path $root 'crates'), (Join-Path $root 'cmd') -Filter *.rs |
    Where-Object { $_.FullName -match '\\src\\' -and $_.Name -ne 'tests.rs' -and $_.Name -notmatch 'test' } |
    ForEach-Object {
        [pscustomobject]@{
            Lines = (Get-Content $_.FullName | Measure-Object -Line).Lines
            Path  = $_.FullName.Substring($root.Length + 1)
        }
    } |
    Where-Object { $_.Lines -gt 600 } |
    Sort-Object -Property Lines -Descending |
    ForEach-Object { Write-Output ("{0}`t{1}" -f $_.Lines, $_.Path) }

Write-Output "== 测试基线请自行运行 cargo test --workspace（本脚本不执行构建） =="
