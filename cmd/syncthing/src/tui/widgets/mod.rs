pub mod header;
pub mod log_line;
pub mod progress;
pub mod status_bar;

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// 在父区域中居中创建一个弹窗区域，自适应终端大小。
///
/// `max_w`/`max_h` 是最大尺寸（字符数）。实际尺寸不会超过终端可用区域。
pub fn centered_popup(max_w: u16, max_h: u16, r: Rect) -> Rect {
    let w = max_w.min(r.width.saturating_sub(4));
    let h = max_h.min(r.height.saturating_sub(4));
    let x = r.x + (r.width.saturating_sub(w)) / 2;
    let y = r.y + (r.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// 在父区域中居中创建一个百分比大小的区域（保留给不需要自适应的地方）
#[allow(dead_code)]
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
