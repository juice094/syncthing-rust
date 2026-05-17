param(
    [Parameter(Mandatory=$true)]
    [string]$SyncDir,
    [string]$Duration = "1h",
    [string]$LogFile = ""
)

$ErrorActionPreference = "Continue"

if (-not (Test-Path $SyncDir)) {
    New-Item -ItemType Directory -Path $SyncDir -Force | Out-Null
}

if ($LogFile -eq "") {
    $LogFile = "$SyncDir\..\churn.log"
}

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$maxDuration = if ($Duration -match "^(\d+)([hms])$") {
    $val = [int]$Matches[1]
    switch ($Matches[2]) {
        "s" { [TimeSpan]::FromSeconds($val) }
        "m" { [TimeSpan]::FromMinutes($val) }
        "h" { [TimeSpan]::FromHours($val) }
        default { [TimeSpan]::FromHours(72) }
    }
} else { [TimeSpan]::FromHours(72) }

$sizes = @(1024, 64*1024, 1024*1024, 10*1024*1024)
$counter = 0

Add-Content -Path $LogFile -Value "[churn] Starting in $SyncDir, duration=$Duration"

while ($sw.Elapsed -lt $maxDuration) {
    $counter++
    $size = $sizes[($counter - 1) % $sizes.Length]
    $file = Join-Path $SyncDir ("file_{0:D4}.dat" -f $counter)
    $data = New-Object byte[] $size
    [System.Random]::new().NextBytes($data)
    [System.IO.File]::WriteAllBytes($file, $data)
    Add-Content -Path $LogFile -Value ("[churn] CREATE {0} ({1} bytes)" -f (Split-Path $file -Leaf), $size)

    if ($counter -gt 3) {
        $oldFile = Join-Path $SyncDir ("file_{0:D4}.dat" -f ($counter - 3))
        if (Test-Path $oldFile) {
            $oldSize = $sizes[($counter - 4) % $sizes.Length]
            $oldData = New-Object byte[] $oldSize
            [System.Random]::new($counter).NextBytes($oldData)
            [System.IO.File]::WriteAllBytes($oldFile, $oldData)
            Add-Content -Path $LogFile -Value ("[churn] MODIFY {0}" -f (Split-Path $oldFile -Leaf))
        }
    }

    if ($counter -gt 6) {
        $oldFile = Join-Path $SyncDir ("file_{0:D4}.dat" -f ($counter - 6))
        if (Test-Path $oldFile) {
            Remove-Item $oldFile -Force
            Add-Content -Path $LogFile -Value ("[churn] DELETE {0}" -f (Split-Path $oldFile -Leaf))
        }
    }

    Start-Sleep -Seconds 30
}

Add-Content -Path $LogFile -Value "[churn] Duration reached, stopping."
