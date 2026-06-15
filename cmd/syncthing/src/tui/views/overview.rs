use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use syncthing_core::types::FolderStatus;

use crate::tui::app::App;
use crate::tui::widgets::progress;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // 设备信息
            Constraint::Length(6), // Sync Status 总览卡片
            Constraint::Fill(1),   // 日志：占据剩余全部空间
        ])
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

    // Sync Status 总览卡片
    let sync_area = chunks[1];
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .margin(1)
        .split(sync_area);

    let block = Block::default().borders(Borders::ALL).title("Sync Status");
    f.render_widget(block, sync_area);

    // 收集非 Idle 的活跃 folder
    let active_folders: Vec<(&String, &FolderStatus)> = app
        .folder_states
        .iter()
        .filter(|(_, s)| !matches!(s, FolderStatus::Idle))
        .collect();

    // 总体同步进度：非 Idle folder 的进度平均
    let overall_ratio = if active_folders.is_empty() {
        1.0
    } else {
        let sum: f64 = active_folders
            .iter()
            .map(|(folder, _)| {
                app.sync_progress
                    .get(folder.as_str())
                    .cloned()
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0)
            })
            .sum();
        (sum / active_folders.len() as f64).clamp(0.0, 1.0)
    };

    // 第 1 行：Overall sync
    let label_col = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(14), Constraint::Min(10)])
        .split(inner[0]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Overall sync: ",
            theme.style_header,
        ))),
        label_col[0],
    );
    progress::draw_gauge(
        f,
        label_col[1],
        theme,
        &format!("{:.0}%", overall_ratio * 100.0),
        overall_ratio,
    );

    // 第 2 行：Devices online
    let online_count = app.connected_devices.len();
    let total_devices = app.config.devices.len();
    let online_style = if online_count == total_devices && total_devices > 0 {
        theme.style_online
    } else if online_count == 0 {
        theme.style_offline
    } else {
        Style::default().fg(theme.text_primary)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Devices online: ", theme.style_header),
            Span::styled(
                format!("{} / {}", online_count, total_devices),
                online_style,
            ),
        ])),
        inner[1],
    );

    // 第 3 行：Folders active / All folders up to date
    let active_line = if active_folders.is_empty() {
        Line::from(Span::styled(
            "All folders up to date",
            Style::default().fg(theme.success),
        ))
    } else {
        let mut counts: Vec<String> = Vec::new();
        let scanning = active_folders
            .iter()
            .filter(|(_, s)| matches!(s, FolderStatus::Scanning))
            .count();
        let scan_waiting = active_folders
            .iter()
            .filter(|(_, s)| matches!(s, FolderStatus::ScanWaiting))
            .count();
        let pulling = active_folders
            .iter()
            .filter(|(_, s)| matches!(s, FolderStatus::Pulling))
            .count();
        let pushing = active_folders
            .iter()
            .filter(|(_, s)| matches!(s, FolderStatus::Pushing))
            .count();
        let sync_waiting = active_folders
            .iter()
            .filter(|(_, s)| matches!(s, FolderStatus::SyncWaiting))
            .count();

        if scanning + scan_waiting > 0 {
            counts.push(format!("{} scanning", scanning + scan_waiting));
        }
        if pulling > 0 {
            counts.push(format!("{} pulling", pulling));
        }
        if pushing > 0 {
            counts.push(format!("{} pushing", pushing));
        }
        if sync_waiting > 0 {
            counts.push(format!("{} waiting", sync_waiting));
        }

        Line::from(vec![
            Span::styled("Folders active: ", theme.style_header),
            Span::raw(counts.join(", ")),
        ])
    };
    f.render_widget(Paragraph::new(active_line), inner[2]);

    // 第 4 行：Last update
    let last_update_text = if let Some(instant) = app.last_sync_progress_update {
        let secs = instant.elapsed().as_secs();
        if secs < 1 {
            "just now".to_string()
        } else {
            format!("{}s ago", secs)
        }
    } else {
        "never".to_string()
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Last update: ", theme.style_header),
            Span::raw(last_update_text),
        ])),
        inner[3],
    );

    // Recent Logs
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
    f.render_widget(logs_para, chunks[2]);
}
