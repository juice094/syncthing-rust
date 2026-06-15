use ratatui::{
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

use crate::tui::app::App;

fn folder_status_text(
    status: &syncthing_core::types::FolderStatus,
    theme: &crate::tui::theme::Theme,
) -> (&'static str, Style) {
    match status {
        syncthing_core::types::FolderStatus::Idle => ("● Idle", Style::default().fg(theme.success)),
        syncthing_core::types::FolderStatus::ScanWaiting => {
            ("⏸ ScanWait", Style::default().fg(theme.warning))
        }
        syncthing_core::types::FolderStatus::Scanning => {
            ("⟳ Scanning", Style::default().fg(theme.warning))
        }
        syncthing_core::types::FolderStatus::SyncWaiting => {
            ("⟳ SyncWait", Style::default().fg(theme.info))
        }
        syncthing_core::types::FolderStatus::Pulling => {
            ("⟳ Pulling", Style::default().fg(theme.info))
        }
        syncthing_core::types::FolderStatus::Pushing => {
            ("⟳ Pushing", Style::default().fg(theme.info))
        }
        syncthing_core::types::FolderStatus::Synced => {
            ("✓ Synced", Style::default().fg(theme.success))
        }
        syncthing_core::types::FolderStatus::Paused => {
            ("⏸ Paused", Style::default().fg(theme.muted))
        }
        syncthing_core::types::FolderStatus::Error => ("✗ Error", Style::default().fg(theme.error)),
    }
}

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let theme = &app.theme;

    let title = if let Some(q) = &app.list_filter {
        format!("Folders (filtered: '{q}') [↑↓ navigate, Enter edit, i ignore, d delete]")
    } else {
        "Folders (Ins/a: add, Enter/e: edit, i: ignore, Del/d: delete)".to_string()
    };

    let rows: Vec<Row> = app
        .folder_filter_matches
        .iter()
        .copied()
        .filter_map(|idx| app.config.folders.get(idx))
        .map(|fo| {
            let devs = fo
                .devices
                .iter()
                .map(|id| id.to_string().split('-').next().unwrap_or("").to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let (status_label, status_style) = app
                .folder_states
                .get(&fo.id)
                .map(|s| folder_status_text(s, theme))
                .unwrap_or(("? Unknown", Style::default().fg(theme.text_secondary)));
            Row::new(vec![
                Cell::from(Span::styled(fo.id.clone(), theme.style_header)),
                Cell::from(Span::styled(fo.path.clone(), theme.style_idle)),
                Cell::from(Span::styled(devs, theme.style_idle)),
                Cell::from(Span::styled(status_label, status_style)),
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
    .block(Block::default().borders(Borders::ALL).title(title.as_str()))
    .row_highlight_style(theme.style_header.add_modifier(Modifier::REVERSED));

    let mut state = ratatui::widgets::TableState::default();
    state.select(if app.folder_filter_matches.is_empty() {
        None
    } else {
        Some(app.list_filter_selected)
    });
    f.render_stateful_widget(table, area, &mut state);
}
