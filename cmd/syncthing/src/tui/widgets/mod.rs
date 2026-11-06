/// Tab header widget.
pub mod header;
/// Colored log line parsing.
pub mod log_line;
/// Progress bar widgets.
pub mod progress;
/// Bottom status bar widget.
pub mod status_bar;

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Creates a centered popup rectangle within `r`, shrinking to fit the terminal.
///
/// `max_w`/`max_h` are the maximum sizes in cells; the result never exceeds
/// the available area.
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

/// Creates a percentage-sized centered rectangle (kept for non-adaptive call sites).
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
