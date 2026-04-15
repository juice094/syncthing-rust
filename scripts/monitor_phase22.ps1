$log = "C:\Users\22414\dev\third_party\syncthing-rust\phase22_monitor.log"
"=== Phase 2.2 Long-running test started at $(Get-Date) ===" | Out-File $log

for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Seconds 120
    $ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    try {
        $status = Invoke-RestMethod -Uri "http://127.0.0.1:8385/rest/system/status" -Method GET -TimeoutSec 10
        $conn = Invoke-RestMethod -Uri "http://127.0.0.1:8385/rest/connections" -Method GET -TimeoutSec 10
        $proc = Get-Process syncthing -ErrorAction SilentlyContinue | Select-Object -First 1
        $mem = if ($proc) { "{0:N2}" -f ($proc.WorkingSet64 / 1MB) } else { "N/A" }
        $cpu = if ($proc) { "{0:N2}" -f ($proc.TotalProcessorTime.TotalSeconds) } else { "N/A" }
        "$ts | uptime=$($status.uptime)s mem=${mem}MB cpu=${cpu}s devices=$($status.device_count)" | Out-File $log -Append

        if ($conn.total -gt 0) {
            $conn.connections | ForEach-Object {
                "$ts | conn: $($_.id) connected=true addr=$($_.address) type=$($_.conn_type)" | Out-File $log -Append
            }
        } else {
            "$ts | conn: no connections" | Out-File $log -Append
        }
    } catch {
        "$ts | ERROR: $_" | Out-File $log -Append
    }
}

"=== Phase 2.2 Long-running test ended at $(Get-Date) ===" | Out-File $log -Append
