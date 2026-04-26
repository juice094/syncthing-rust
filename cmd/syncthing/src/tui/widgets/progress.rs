//! 同步进度条组件

#![allow(dead_code)]

//!
//! 基于 ratatui::widgets::Gauge，使用 theme 配色。
//! 支持百分比和字节双显示模式。

use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::{Gauge, LineGauge},
    Frame,
};

use crate::tui::theme::Theme;

/// 将字节数格式化为人类可读字符串
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exp = (bytes as f64).log(1024.0).min(UNITS.len() as f64 - 1.0) as usize;
    let value = bytes as f64 / 1024f64.powi(exp as i32);
    if exp == 0 {
        format!("{} {}", bytes, UNITS[exp])
    } else {
        format!("{:.2} {}", value, UNITS[exp])
    }
}

/// 标准进度条（带百分比标签）
pub fn draw_gauge(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    theme: &Theme,
    label: &str,
    ratio: f64,
) {
    let ratio = ratio.clamp(0.0, 1.0);
    let gauge = Gauge::default()
        .block(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(label),
        )
        .gauge_style(Style::default().fg(theme.primary).bg(theme.surface))
        .ratio(ratio)
        .label(format!("{:.0}%", ratio * 100.0));
    f.render_widget(gauge, area);
}

/// 紧凑行内进度条（适合列表中的文件夹行）
pub fn draw_line_gauge(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    theme: &Theme,
    ratio: f64,
) {
    let ratio = ratio.clamp(0.0, 1.0);
    let gauge = LineGauge::default()
        .filled_style(Style::default().fg(theme.primary))
        .unfilled_style(Style::default().fg(theme.border))
        .ratio(ratio)
        .label(format!("{:.0}%", ratio * 100.0));
    f.render_widget(gauge, area);
}

/// 文件夹同步状态行（名称 + 进度条 + 速率）
pub fn folder_sync_line<'a>(
    name: &'a str,
    completion: f64,
    in_rate: Option<u64>,
    out_rate: Option<u64>,
    theme: &Theme,
) -> Line<'a> {
    let mut spans = vec![
        Span::raw(format!("{} ", name)),
        Span::styled(
            format!("{:.0}%", completion.clamp(0.0, 100.0)),
            Style::default().fg(if completion >= 100.0 {
                theme.success
            } else {
                theme.primary
            }),
        ),
    ];

    if let Some(rate) = in_rate {
        spans.push(Span::raw(" ↓"));
        spans.push(Span::styled(format_bytes(rate), Style::default().fg(theme.info)));
        spans.push(Span::raw("/s"));
    }
    if let Some(rate) = out_rate {
        spans.push(Span::raw(" ↑"));
        spans.push(Span::styled(format_bytes(rate), Style::default().fg(theme.info)));
        spans.push(Span::raw("/s"));
    }

    Line::from(spans)
}
