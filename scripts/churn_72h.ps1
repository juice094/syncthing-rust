# 72h stress-test churn script for syncthing-rust
# Creates/modifies/deletes files in the target sync folder at regular intervals.

param(
    [string]$Folder = "C:\Users\22414\.syncthing-stress-test\72h-data",
    [string]$Duration = "72h",
    [int]$IntervalSec = 300,
    [string]$LogFile = "C:\Users\22414\.syncthing-stress-test\churn.log",
    [int]$MaxFiles = 1000
)

$ErrorActionPreference = "Continue"

function Write-Log {
    param([string]$Message)
    $line = "$(Get-Date -Format 'yyyy-MM-ddTHH:mm:ssZ')  $Message"
    Add-Content -Path $LogFile -Value $line -ErrorAction SilentlyContinue
    Write-Host $line
}

function Random-Bytes {
    param([int]$Size)
    $bytes = New-Object byte[] $Size
    (New-Object System.Random).NextBytes($bytes)
    return $bytes
}

function Parse-Duration {
    param([string]$s)
    $s = $s.Trim()
    if ($s -match '^\d+$') { return [TimeSpan]::FromSeconds([int]$s) }
    if ($s -match '^(\d+)([smhd])$') {
        $num = [int]$Matches[1]
        switch ($Matches[2]) {
            's' { return [TimeSpan]::FromSeconds($num) }
            'm' { return [TimeSpan]::FromMinutes($num) }
            'h' { return [TimeSpan]::FromHours($num) }
            'd' { return [TimeSpan]::FromDays($num) }
        }
    }
    throw "Invalid duration format: $s (expected e.g. 72h, 5m, 3600)"
}

if (-not (Test-Path $Folder)) {
    New-Item -ItemType Directory -Path $Folder -Force | Out-Null
}
$Folder = (Resolve-Path $Folder).Path

$durationSpan = Parse-Duration $Duration
$startTime = Get-Date
$endTime = $startTime.Add($durationSpan)
$counter = 0
$rnd = New-Object System.Random

Write-Log "Churn started: folder=$Folder duration=$Duration interval=${IntervalSec}s"

while ((Get-Date) -lt $endTime) {
    $counter++
    $elapsed = [math]::Floor(((Get-Date) - $startTime).TotalMinutes)

    # 1. Create a new file (mix of small text and medium binary)
    $size = switch ($rnd.Next(5)) {
        0 { 256 }
        1 { 4096 }
        2 { 65536 }
        3 { 524288 }
        default { 1048576 }
    }
    $fileName = "churn_{0:D8}_{1:D7}.bin" -f $counter, $size
    $filePath = Join-Path $Folder $fileName
    try {
        [System.IO.File]::WriteAllBytes($filePath, (Random-Bytes $size))
        Write-Log "CREATE $fileName size=$size elapsed_min=$elapsed"
    } catch {
        Write-Log "ERROR creating $fileName : $_"
    }

    # 2. Modify a recent older file (append a few bytes)
    $modTarget = Join-Path $Folder ("churn_{0:D8}_*.bin" -f ($counter - 3))
    $modFile = Get-ChildItem -Path $modTarget -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($modFile) {
        try {
            Add-Content -Path $modFile.FullName -Value (Get-Random) -NoNewline -ErrorAction Stop
            Write-Log "MODIFY $($modFile.Name)"
        } catch {
            Write-Log "ERROR modifying $($modFile.Name) : $_"
        }
    }

    # 3. Rename an older file
    $renTarget = Join-Path $Folder ("churn_{0:D8}_*.bin" -f ($counter - 6))
    $renFile = Get-ChildItem -Path $renTarget -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($renFile) {
        $newName = "renamed_{0:D8}_$($renFile.Name)" -f $counter
        $newPath = Join-Path $Folder $newName
        try {
            Move-Item -Path $renFile.FullName -Destination $newPath -Force -ErrorAction Stop
            Write-Log "RENAME $($renFile.Name) -> $newName"
        } catch {
            Write-Log "ERROR renaming $($renFile.Name) : $_"
        }
    }

    # 4. Bound total churn file count
    $churnFiles = Get-ChildItem -Path $Folder -Filter "churn_*.bin" -ErrorAction SilentlyContinue |
                  Sort-Object LastWriteTime
    if ($churnFiles.Count -gt $MaxFiles) {
        $delFile = $churnFiles | Select-Object -First 1
        try {
            [System.IO.File]::Delete($delFile.FullName)
            Write-Log "DELETE $($delFile.Name)"
        } catch {
            Write-Log "ERROR deleting $($delFile.Name) : $_"
        }
    }

    Start-Sleep -Seconds $IntervalSec
}

Write-Log "Churn completed after $Duration"
