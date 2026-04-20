use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::App;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(0)])
        .split(area);

    let device_id = app
        .config
        .local_device_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let theme = &app.theme;
    let text = Text::from(vec![
        Line::from(vec![
            Span::styled("Device ID: ", theme.style_header),
            Span::raw(device_id),
        ]),
        Line::from(vec![
            Span::styled("Name:      ", theme.style_header),
            Span::raw(&app.device_name),
        ]),
        Line::from(vec![
            Span::styled("Listen:    ", theme.style_header),
            Span::raw(&app.listen),
        ]),
        Line::from(vec![
            Span::styled("Folders:   ", theme.style_header),
            Span::raw(app.config.folders.len().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Devices:   ", theme.style_header),
            Span::raw(app.config.devices.len().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Connected: ", theme.style_header),
            Span::styled(
                app.connected_devices.len().to_string(),
                if app.connected_devices.is_empty() {
                    theme.style_offline
                } else {
                    theme.style_online
                },
            ),
        ]),
    ]);

    let para = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Overview"))
        .wrap(Wrap { trim: true });
    f.render_widget(para, chunks[0]);

    let logs: Text = app
        .log_lines
        .iter()
        .rev()
        .take(10)
        .map(|l| Line::raw(l.as_str()))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .into();

    let logs_para = Paragraph::new(logs)
        .block(Block::default().borders(Borders::ALL).title("Recent Logs"))
        .wrap(Wrap { trim: true });
    f.render_widget(logs_para, chunks[1]);
}
