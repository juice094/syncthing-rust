# 启动 TUI 并设置较小的控制台窗口，以便验证底部状态栏

$exe = "C:\Users\22414\dev\syncthing-rust\target\release\syncthing.exe"
$config = "C:\Users\22414\AppData\Local\syncthing-rust"

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $exe
$psi.Arguments = "tui --config-dir `"$config`""
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $false

$p = [System.Diagnostics.Process]::Start($psi)
Start-Sleep -Milliseconds 500

# 调整窗口大小（Windows 控制台窗口）
Add-Type @"
using System;
using System.Runtime.InteropServices;

public class WinConsole {
    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);

    public static readonly IntPtr HWND_TOP = IntPtr.Zero;
    public const uint SWP_SHOWWINDOW = 0x0040;
}
"@

[WinConsole]::SetWindowPos($p.MainWindowHandle, [WinConsole]::HWND_TOP, 100, 100, 1200, 600, [WinConsole]::SWP_SHOWWINDOW)
Write-Output $p.Id
