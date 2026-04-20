//! 动画加载指示器
//!
//! 基于 frame_counter 实现，不依赖额外定时器。
//! 在 mod.rs 的 250ms tick 中递增 frame_counter，
//! spinner 根据 counter 计算当前帧。

use ratatui::{
    style::Style,
    text::{Line, Span},
};

/// Unicode Braille 点阵 spinner 帧序列
const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 每 N 帧（250ms tick）切换一次 spinner 图案
const TICKS_PER_FRAME: u64 = 1;

pub struct Spinner {
    label: String,
    style: Style,
}

impl Spinner {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: Style::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// 渲染当前 spinner 帧
    pub fn render(&self, frame_counter: u64) -> Line<'_> {
        let idx = ((frame_counter / TICKS_PER_FRAME) as usize) % FRAMES.len();
        let symbol = FRAMES[idx];
        Line::from(vec![
            Span::styled(symbol.to_string(), self.style),
            Span::raw(" "),
            Span::styled(self.label.clone(), self.style),
        ])
    }

    /// 纯符号，不带标签（用于紧凑场景）
    pub fn symbol(frame_counter: u64) -> &'static str {
        let idx = ((frame_counter / TICKS_PER_FRAME) as usize) % FRAMES.len();
        FRAMES[idx]
    }
}

/// 脉冲状态点（用于设备在线/同步状态）
pub fn pulse_dot(frame_counter: u64, active: bool) -> &'static str {
    if !active {
        return "○";
    }
    // 每 8 帧（2秒）一个脉冲周期
    let phase = (frame_counter % 8) as usize;
    ["●", "◐", "◑", "◒", "◓", "◒", "◑", "◐"][phase]
}
