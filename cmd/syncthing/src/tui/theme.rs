//! TUI theme system: centralizes colors and styles.
//!
//! All drawing code takes colors from `Theme`; hard-coded `Color::...` values
//! are forbidden. New themes only need a `Theme::new_xxx()` constructor.
// NOTE: Theme variants reserved for future TUI enhancements (v0.3.0+)
#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};

/// Semantic theme definition.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,

    // === semantic colors ===
    pub primary: Color,        // brand (focus, progress, titles)
    pub secondary: Color,      // accent (highlights, links)
    pub success: Color,        // success / online
    pub warning: Color,        // warning / scanning
    pub error: Color,          // error / offline
    pub info: Color,           // informational
    pub muted: Color,          // secondary text / timestamps
    pub surface: Color,        // panel background
    pub border: Color,         // plain border
    pub border_focused: Color, // focused border
    pub text_primary: Color,   // primary text
    pub text_secondary: Color, // secondary text

    // === precomputed styles (avoid rebuilding per frame) ===
    pub style_online: Style,
    pub style_offline: Style,
    pub style_syncing: Style,
    pub style_scanning: Style,
    pub style_idle: Style,
    pub style_error: Style,
    pub style_header: Style,
    pub style_status_bg: Style,
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
    /// Official Syncthing dark theme (optimized for 256-color terminals).
    pub fn syncthing_dark() -> Self {
        let primary = Color::Rgb(40, 100, 200); // #2864C8
        let secondary = Color::Rgb(80, 160, 220); // #50A0DC
        let success = Color::Rgb(100, 200, 100); // #64C864
        let warning = Color::Rgb(240, 180, 60); // #F0B43C
        let error = Color::Rgb(220, 80, 80); // #DC5050
        let info = Color::Rgb(120, 180, 240); // #78B4F0
        let muted = Color::Rgb(120, 120, 120); // #787878
        let surface = Color::Rgb(30, 30, 35); // #1E1E23
        let border = Color::Rgb(60, 60, 70); // #3C3C46
        let border_focused = primary;
        let text_primary = Color::Rgb(230, 230, 230); // #E6E6E6
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
            style_status_bg: Style::default().bg(surface),
            style_popup_border: Style::default().fg(border_focused),
            style_log_trace: Style::default().fg(Color::Rgb(100, 100, 100)),
            style_log_debug: Style::default().fg(info),
            style_log_info: Style::default().fg(text_primary),
            style_log_warn: Style::default().fg(warning),
            style_log_error: Style::default().fg(error).add_modifier(Modifier::BOLD),
        }
    }

    /// Returns the style matching a folder status.
    pub fn folder_status_style(&self, status: &str) -> Style {
        match status {
            "syncing" | "pulling" => self.style_syncing,
            "scanning" => self.style_scanning,
            "idle" => self.style_idle,
            "error" => self.style_error,
            _ => Style::default().fg(self.text_secondary),
        }
    }

    /// Returns the style matching a log level.
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
