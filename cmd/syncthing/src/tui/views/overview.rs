use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Color,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::App;
use crate::tui::widgets::progress;

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
        constraints.push(Constraint::Length(
            (active_folders.len().max(app.sync_progress.len()) + 1) as u16 + 1,
        ));
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

    // 活跃同步状态区域（文本 + LineGauge 进度条）
    if !active_folders.is_empty() || !app.sync_progress.is_empty() {
        let theme = &app.theme;
        let sync_area = chunks[chunk_idx];

        // 内部垂直布局：标题 + 每文件夹一行
        let row_count = active_folders.len().max(1) + 1; // +1 for header
        let inner_constraints: Vec<Constraint> = std::iter::once(Constraint::Length(1))
            .chain(std::iter::repeat_n(Constraint::Length(1), row_count - 1))
            .collect();
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints(inner_constraints)
            .margin(1) // 1-cell margin inside the block border
            .split(sync_area);

        // 渲染边框
        let block = Block::default().borders(Borders::ALL).title("Sync Status");
        f.render_widget(block, sync_area);

        // 标题行
        f.render_widget(
            Paragraph::new(Text::from(vec![Line::from(Span::styled(
                "Active sync tasks:",
                theme.style_header,
            ))])),
            inner[0],
        );

        // 每文件夹一行：标签文本 + LineGauge
        for (idx, (folder, status)) in active_folders.iter().enumerate() {
            let row = inner.get(idx + 1).copied().unwrap_or(sync_area);

            let label = match status {
                syncthing_core::types::FolderStatus::Scanning => "Scanning",
                syncthing_core::types::FolderStatus::Pulling => "Pulling",
                syncthing_core::types::FolderStatus::Pushing => "Pushing",
                syncthing_core::types::FolderStatus::SyncWaiting => "SyncWait",
                syncthing_core::types::FolderStatus::ScanWaiting => "ScanWait",
                _ => "Working",
            };
            let color = match status {
                syncthing_core::types::FolderStatus::Scanning
                | syncthing_core::types::FolderStatus::ScanWaiting => Color::Yellow,
                syncthing_core::types::FolderStatus::Pulling
                | syncthing_core::types::FolderStatus::Pushing
                | syncthing_core::types::FolderStatus::SyncWaiting => Color::Cyan,
                _ => Color::Gray,
            };
            let progress_ratio = app
                .sync_progress
                .get(folder.as_str())
                .cloned()
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);

            // 水平分割：左侧标签（20宽），右侧进度条
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(20), Constraint::Min(10)])
                .split(row);

            let label_text = Text::from(vec![Line::from(vec![
                Span::raw(format!("{} ", folder)),
                Span::styled(label, ratatui::style::Style::default().fg(color)),
            ])]);
            f.render_widget(Paragraph::new(label_text), cols[0]);

            progress::draw_line_gauge(f, cols[1], theme, progress_ratio);
        }
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
