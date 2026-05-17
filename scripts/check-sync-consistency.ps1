#Requires -Version 5.1
<#
.SYNOPSIS
    Check sync consistency between local and remote sync directories.

.DESCRIPTION
    Compares file counts and optionally hashes between local and remote
    sync folders. Can be run periodically during the two-node stress test.

.PARAMETER LocalDir
    Local sync directory.

.PARAMETER RemoteDir
    Remote sync directory (mounted path, e.g. via SSHFS, or local mirror).

.PARAMETER CheckHashes
    Also compute and compare SHA-256 hashes of all files.
#>
param(
    [Parameter(Mandatory=$true)]
    [string]$LocalDir,

    [Parameter(Mandatory=$true)]
    [string]$RemoteDir,

    [switch]$CheckHashes
)

$ErrorActionPreference = "Stop"

function Get-FileManifest([string]$dir) {
    $manifest = @{}
    Get-ChildItem -Path $dir -File -Recurse -ErrorAction SilentlyContinue | ForEach-Object {
        $relPath = $_.FullName.Substring($dir.Length).TrimStart('\', '/')
        $manifest[$relPath] = @{
            size = $_.Length
            hash = if ($CheckHashes) {
                $sha256 = [System.Security.Cryptography.SHA256]::Create()
                $stream = [System.IO.File]::OpenRead($_.FullName)
                try {
                    $bytes = $sha256.ComputeHash($stream)
                    [BitConverter]::ToString($bytes).Replace("-", "").ToLower()
                } finally {
                    $stream.Close()
                    $sha256.Dispose()
                }
            } else { $null }
        }
    }
    return $manifest
}

Write-Host "Scanning local:  $LocalDir"
$localManifest = Get-FileManifest $LocalDir
Write-Host "Found $($localManifest.Count) files"

Write-Host "Scanning remote: $RemoteDir"
$remoteManifest = Get-FileManifest $RemoteDir
Write-Host "Found $($remoteManifest.Count) files"

$onlyLocal = $localManifest.Keys | Where-Object { $_ -notin $remoteManifest.Keys }
$onlyRemote = $remoteManifest.Keys | Where-Object { $_ -notin $localManifest.Keys }
$common = $localManifest.Keys | Where-Object { $_ -in $remoteManifest.Keys }

$sizeMismatches = @()
$hashMismatches = @()

foreach ($file in $common) {
    $l = $localManifest[$file]
    $r = $remoteManifest[$file]
    if ($l.size -ne $r.size) {
        $sizeMismatches += $file
    } elseif ($CheckHashes -and $l.hash -ne $r.hash) {
        $hashMismatches += $file
    }
}

Write-Host ""
Write-Host "=== Sync Consistency Report ===" -ForegroundColor Cyan
Write-Host "Local files:   $($localManifest.Count)"
Write-Host "Remote files:  $($remoteManifest.Count)"
Write-Host "Common files:  $($common.Count)"
Write-Host ""

if ($onlyLocal.Count -gt 0) {
    Write-Host "Only on local ($($onlyLocal.Count)):" -ForegroundColor Yellow
    $onlyLocal | Select-Object -First 10 | ForEach-Object { Write-Host "  $_" }
    if ($onlyLocal.Count -gt 10) { Write-Host "  ... and $($onlyLocal.Count - 10) more" }
} else {
    Write-Host "Only on local: 0" -ForegroundColor Green
}

if ($onlyRemote.Count -gt 0) {
    Write-Host "Only on remote ($($onlyRemote.Count)):" -ForegroundColor Yellow
    $onlyRemote | Select-Object -First 10 | ForEach-Object { Write-Host "  $_" }
    if ($onlyRemote.Count -gt 10) { Write-Host "  ... and $($onlyRemote.Count - 10) more" }
} else {
    Write-Host "Only on remote: 0" -ForegroundColor Green
}

if ($sizeMismatches.Count -gt 0) {
    Write-Host "Size mismatches ($($sizeMismatches.Count)):" -ForegroundColor Red
    $sizeMismatches | Select-Object -First 10 | ForEach-Object { Write-Host "  $_" }
} else {
    Write-Host "Size mismatches: 0" -ForegroundColor Green
}

if ($CheckHashes -and $hashMismatches.Count -gt 0) {
    Write-Host "Hash mismatches ($($hashMismatches.Count)):" -ForegroundColor Red
    $hashMismatches | Select-Object -First 10 | ForEach-Object { Write-Host "  $_" }
} elseif ($CheckHashes) {
    Write-Host "Hash mismatches: 0" -ForegroundColor Green
}

$isConsistent = ($onlyLocal.Count -eq 0) -and ($onlyRemote.Count -eq 0) -and ($sizeMismatches.Count -eq 0) -and ($hashMismatches.Count -eq 0)
Write-Host ""
if ($isConsistent) {
    Write-Host "Result: CONSISTENT" -ForegroundColor Green
} else {
    Write-Host "Result: INCONSISTENT" -ForegroundColor Red
}
