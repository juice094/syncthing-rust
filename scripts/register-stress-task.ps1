#requires -RunAsAdministrator
<#
.SYNOPSIS
    注册 Windows 计划任务，实现 stress_test 开机自动维系
#>
param(
    [string]$RepoRoot = "C:\Users\22414\dev\third_party\syncthing-rust",
    [string]$TaskName = "syncthing-rust-stress-72h"
)

$daemon = Join-Path $RepoRoot "scripts\stress-daemon.ps1"
$action = New-ScheduledTaskAction -Execute "powershell.exe" `
    -Argument "-ExecutionPolicy Bypass -WindowStyle Hidden -File `"$daemon`""

$triggerBoot = New-ScheduledTaskTrigger -AtStartup
$triggerRepeat = New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval (New-TimeSpan -Minutes 5) -RepetitionDuration (New-TimeSpan -Days 9999)

$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -Hidden

Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger @($triggerBoot, $triggerRepeat) -Principal $principal -Settings $settings -Force
Write-Host "Task '$TaskName' registered. It will start at boot and check every 5 minutes."
