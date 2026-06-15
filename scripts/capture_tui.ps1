# 捕获 syncthing-rust TUI 窗口截图并保存到临时目录。
# 用于自动化验证 TUI 渲染效果。

Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Runtime.InteropServices;
using System.Drawing;
using System.Drawing.Imaging;

public class WindowCapture {
    [DllImport("user32.dll")]
    public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);

    [DllImport("user32.dll")]
    public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int count);

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

    public static string Capture(string title, string outputPath) {
        IntPtr hwnd = FindWindow(null, title);
        if (hwnd == IntPtr.Zero) return "window not found";
        RECT rc;
        if (!GetWindowRect(hwnd, out rc)) return "get rect failed";
        int w = rc.Right - rc.Left;
        int h = rc.Bottom - rc.Top;
        if (w <= 0 || h <= 0) return "invalid size";
        // 放大窗口以确保 120x40 控制台内容完全可见
        SetWindowPos(hwnd, HWND_TOP, 0, 0, 1600, 1000, SWP_SHOWWINDOW);
        System.Threading.Thread.Sleep(500);
        GetWindowRect(hwnd, out rc);
        w = rc.Right - rc.Left;
        h = rc.Bottom - rc.Top;
        using (Bitmap bmp = new Bitmap(w, h)) {
            using (Graphics g = Graphics.FromImage(bmp)) {
                IntPtr hdc = g.GetHdc();
                PrintWindow(hwnd, hdc, 2);
                g.ReleaseHdc(hdc);
            }
            bmp.Save(outputPath, ImageFormat.Png);
        }
        return "captured " + w + "x" + h + " -> " + outputPath;
    }
}
"@

$out = Join-Path $env:TEMP "syncthing-tui-capture.png"
[WindowCapture]::Capture("syncthing-rust TUI", $out)
Write-Output $out
