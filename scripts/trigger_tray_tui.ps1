# 触发 syncthing-rust 托盘图标的双击事件，打开 TUI。
# 用于自动化验证 TUI 启动效果。

Add-Type @"
using System;
using System.Runtime.InteropServices;

public class TrayTrigger {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);

    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);

    public const uint WM_APP = 0x8000;
    public const uint WM_TRAYICON = WM_APP + 1;
    public const uint WM_LBUTTONDBLCLK = 0x0203;

    public static bool Trigger(string className) {
        IntPtr hwnd = FindWindow(className, null);
        if (hwnd == IntPtr.Zero) return false;
        return PostMessage(hwnd, WM_TRAYICON, IntPtr.Zero, (IntPtr)WM_LBUTTONDBLCLK);
    }
}
"@

[TrayTrigger]::Trigger("SyncthingTrayV2")
