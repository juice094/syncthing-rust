//! TUI 主题系统 — 集中管理配色与样式
#![allow(dead_code)]
//!
//! 所有绘制代码通过 Theme 获取颜色，禁止硬编码 Color::Cyan 等。
//! 新增主题只需实现 Theme::new_xxx() 方法。

use ratatui::style::{Color, Modifier, Style};

/// 语义化主题定义
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,

    // === 语义颜色 ===
    pub primary: Color,       // 品牌色（聚焦、进度条、标题）
    pub secondary: Color,     // 辅助色（高亮、链接）
    pub success: Color,       // 成功 / 在线
    pub warning: Color,       // 警告 / 扫描中
    pub error: Color,         // 错误 / 离线
    pub info: Color,          // 信息提示
    pub muted: Color,         // 次要文本 / 时间戳
    pub surface: Color,       // 面板背景
    pub border: Color,        // 普通边框
    pub border_focused: Color, // 聚焦边框
    pub text_primary: Color,   // 主要文本
    pub text_secondary: Color, // 次要文本

    // === 预计算样式（避免运行时重复构造） ===
    pub style_online: Style,
    pub style_offline: Style,
    pub style_syncing: Style,
    pub style_scanning: Style,
    pub style_idle: Style,
    pub style_error: Style,
    pub style_header: Style,
    pub style_popup_border: Style,
    pub style_log_trace: Style,
    pub style_log_debug: Style,
    pub style_log_info: Style,
    pub style_log_warn: Style,
    pub style_log_error: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self::syncthing_dark()
    }
}

impl Theme {
    /// Syncthing 官方暗色主题（终端 256 色优化版）
    pub fn syncthing_dark() -> Self {
        let primary = Color::Rgb(40, 100, 200);       // #2864C8
        let secondary = Color::Rgb(80, 160, 220);     // #50A0DC
        let success = Color::Rgb(100, 200, 100);      // #64C864
        let warning = Color::Rgb(240, 180, 60);       // #F0B43C
        let error = Color::Rgb(220, 80, 80);          // #DC5050
        let info = Color::Rgb(120, 180, 240);         // #78B4F0
        let muted = Color::Rgb(120, 120, 120);        // #787878
        let surface = Color::Rgb(30, 30, 35);         // #1E1E23
        let border = Color::Rgb(60, 60, 70);          // #3C3C46
        let border_focused = primary;
        let text_primary = Color::Rgb(230, 230, 230);  // #E6E6E6
        let text_secondary = Color::Rgb(160, 160, 160); // #A0A0A0

        Self {
            name: "Syncthing Dark",
            primary,
            secondary,
            success,
            warning,
            error,
            info,
            muted,
            surface,
            border,
            border_focused,
            text_primary,
            text_secondary,

            style_online: Style::default().fg(success).add_modifier(Modifier::BOLD),
            style_offline: Style::default().fg(error),
            style_syncing: Style::default().fg(warning).add_modifier(Modifier::BOLD),
            style_scanning: Style::default().fg(secondary),
            style_idle: Style::default().fg(text_secondary),
            style_error: Style::default().fg(error).add_modifier(Modifier::BOLD),
            style_header: Style::default()
                .fg(text_primary)
                .add_modifier(Modifier::BOLD),
            style_popup_border: Style::default().fg(border_focused),
            style_log_trace: Style::default().fg(Color::Rgb(100, 100, 100)),
            style_log_debug: Style::default().fg(info),
            style_log_info: Style::default().fg(text_primary),
            style_log_warn: Style::default().fg(warning),
            style_log_error: Style::default().fg(error).add_modifier(Modifier::BOLD),
        }
    }

    /// 根据文件夹状态返回对应样式
    pub fn folder_status_style(&self, status: &str) -> Style {
        match status {
            "syncing" | "pulling" => self.style_syncing,
            "scanning" => self.style_scanning,
            "idle" => self.style_idle,
            "error" => self.style_error,
            _ => Style::default().fg(self.text_secondary),
        }
    }

    /// 根据日志级别返回对应样式
    pub fn log_level_style(&self, level: &str) -> Style {
        match level.to_uppercase().as_str() {
            "TRACE" => self.style_log_trace,
            "DEBUG" => self.style_log_debug,
            "INFO" => self.style_log_info,
            "WARN" | "WARNING" => self.style_log_warn,
            "ERROR" => self.style_log_error,
            _ => self.style_log_info,
        }
    }
}

