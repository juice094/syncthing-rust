use ratatui::{
    style::{Color, Modifier},
    text::Span,
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

use crate::tui::app::App;

fn folder_status_text(status: &syncthing_core::types::FolderStatus) -> (&'static str, Color) {
    match status {
        syncthing_core::types::FolderStatus::Idle => ("Idle", Color::Green),
        syncthing_core::types::FolderStatus::ScanWaiting => ("ScanWait", Color::Yellow),
        syncthing_core::types::FolderStatus::Scanning => ("Scanning", Color::Yellow),
        syncthing_core::types::FolderStatus::SyncWaiting => ("SyncWait", Color::Cyan),
        syncthing_core::types::FolderStatus::Pulling => ("Pulling", Color::Cyan),
        syncthing_core::types::FolderStatus::Pushing => ("Pushing", Color::Cyan),
        syncthing_core::types::FolderStatus::Synced => ("Synced", Color::Green),
        syncthing_core::types::FolderStatus::Paused => ("Paused", Color::DarkGray),
        syncthing_core::types::FolderStatus::Error => ("Error", Color::Red),
    }
}

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let theme = &app.theme;
    let rows: Vec<Row> = app
        .config
        .folders
        .iter()
        .map(|fo| {
            let devs = fo
                .devices
                .iter()
                .map(|id| id.to_string().split('-').next().unwrap_or("").to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let (status_label, status_color) = app
                .folder_states
                .get(&fo.id)
                .map(|s| folder_status_text(s))
                .unwrap_or(("Unknown", Color::Gray));
            Row::new(vec![
                Cell::from(Span::styled(fo.id.clone(), theme.style_header)),
                Cell::from(Span::styled(fo.path.clone(), theme.style_idle)),
                Cell::from(Span::styled(devs, theme.style_idle)),
                Cell::from(Span::styled(status_label, ratatui::style::Style::default().fg(status_color))),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            ratatui::layout::Constraint::Percentage(20),
            ratatui::layout::Constraint::Percentage(40),
            ratatui::layout::Constraint::Percentage(25),
            ratatui::layout::Constraint::Percentage(15),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from(Span::styled("ID", theme.style_header)),
            Cell::from(Span::styled("Path", theme.style_header)),
            Cell::from(Span::styled("Devices", theme.style_header)),
            Cell::from(Span::styled("State", theme.style_header)),
        ])
        .style(Modifier::BOLD),
    )
    .block(Block::default().borders(Borders::ALL).title("Folders (a: add, d: delete)"))
    .row_highlight_style(theme.style_header.add_modifier(Modifier::REVERSED));

    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(app.folder_selected));
    f.render_stateful_widget(table, area, &mut state);
}
