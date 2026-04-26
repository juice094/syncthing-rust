use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Color,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::App;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let mut constraints = vec![
        Constraint::Length(6), // 设备信息
    ];

    // 如果有活跃的同步状态，预留空间
    let active_folders: Vec<(&String, &syncthing_core::types::FolderStatus)> = app
        .folder_states
        .iter()
        .filter(|(_, s)| !matches!(s, syncthing_core::types::FolderStatus::Idle))
        .collect();
    if !active_folders.is_empty() || !app.sync_progress.is_empty() {
        constraints.push(Constraint::Length((active_folders.len().max(app.sync_progress.len()) + 1) as u16 + 1));
    }
    constraints.push(Constraint::Min(0)); // 日志

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
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

    let mut chunk_idx = 1;

    // 活跃同步状态区域
    if !active_folders.is_empty() || !app.sync_progress.is_empty() {
        let theme = &app.theme;
        let mut status_lines = vec![
            Line::from(Span::styled("Active sync tasks:", theme.style_header)),
        ];
        for (folder, status) in &active_folders {
            let label = match status {
                syncthing_core::types::FolderStatus::Scanning => "Scanning",
                syncthing_core::types::FolderStatus::Pulling => "Pulling",
                syncthing_core::types::FolderStatus::Pushing => "Pushing",
                syncthing_core::types::FolderStatus::SyncWaiting => "SyncWait",
                syncthing_core::types::FolderStatus::ScanWaiting => "ScanWait",
                _ => "Working",
            };
            let color = match status {
                syncthing_core::types::FolderStatus::Scanning | syncthing_core::types::FolderStatus::ScanWaiting => Color::Yellow,
                syncthing_core::types::FolderStatus::Pulling | syncthing_core::types::FolderStatus::Pushing | syncthing_core::types::FolderStatus::SyncWaiting => Color::Cyan,
                _ => Color::Gray,
            };
            let progress = app.sync_progress.get(folder.as_str()).cloned().unwrap_or(0.0);
            status_lines.push(Line::from(vec![
                Span::raw(format!("  {}: ", folder)),
                Span::styled(label, ratatui::style::Style::default().fg(color)),
                Span::raw(format!(" {:.0}%", progress * 100.0)),
            ]));
        }
        let status_para = Paragraph::new(Text::from(status_lines))
            .block(Block::default().borders(Borders::ALL).title("Sync Status"))
            .wrap(Wrap { trim: true });
        f.render_widget(status_para, chunks[chunk_idx]);
        chunk_idx += 1;
    }

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
    f.render_widget(logs_para, chunks[chunk_idx]);
}
