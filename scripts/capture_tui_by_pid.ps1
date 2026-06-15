# 根据 PID 捕获 TUI 窗口截图

Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Runtime.InteropServices;
using System.Drawing;
using System.Drawing.Imaging;
using System.Text;

public class WindowCapture {
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out int lpdwProcessId);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int count);

    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);

    public static readonly IntPtr HWND_TOP = IntPtr.Zero;
    public const uint SWP_SHOWWINDOW = 0x0040;

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public static string Capture(int pid, string outputPath) {
        IntPtr found = IntPtr.Zero;
        StringBuilder sb = new StringBuilder(256);
        EnumWindows((hWnd, _) => {
            if (!IsWindowVisible(hWnd)) return true;
            int wpid;
            GetWindowThreadProcessId(hWnd, out wpid);
            if (wpid == pid) {
                GetWindowText(hWnd, sb, 256);
                string title = sb.ToString();
                if (title.Contains("syncthing") || title.Contains("TUI")) {
                    found = hWnd;
                    return false;
                }
            }
            return true;
        }, IntPtr.Zero);

        if (found == IntPtr.Zero) return "window not found";
        RECT rc;
        if (!GetWindowRect(found, out rc)) return "get rect failed";
        int w = rc.Right - rc.Left;
        int h = rc.Bottom - rc.Top;
        SetWindowPos(found, HWND_TOP, 0, 0, 1600, 1000, SWP_SHOWWINDOW);
        System.Threading.Thread.Sleep(500);
        GetWindowRect(found, out rc);
        w = rc.Right - rc.Left;
        h = rc.Bottom - rc.Top;
        using (Bitmap bmp = new Bitmap(w, h)) {
            using (Graphics g = Graphics.FromImage(bmp)) {
                IntPtr hdc = g.GetHdc();
                PrintWindow(found, hdc, 2);
                g.ReleaseHdc(hdc);
            }
            bmp.Save(outputPath, ImageFormat.Png);
        }
        return "captured " + w + "x" + h + " -> " + outputPath;
    }
}
"@

$proc = Get-CimInstance Win32_Process -Filter "Name='syncthing.exe'" | Where-Object { $_.CommandLine -like '*tui*' } | Select-Object -First 1
if (-not $proc) {
    Write-Error "TUI process not found"
    exit 1
}

$out = Join-Path $env:TEMP "syncthing-tui-capture-pid.png"
[WindowCapture]::Capture([int]$proc.ProcessId, $out)
Write-Output $out
