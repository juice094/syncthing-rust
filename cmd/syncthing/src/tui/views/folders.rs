use ratatui::{
    style::{Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

use crate::tui::app::App;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
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
            Row::new(vec![
                Cell::from(fo.id.clone()),
                Cell::from(fo.path.clone()),
                Cell::from(devs),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            ratatui::layout::Constraint::Percentage(25),
            ratatui::layout::Constraint::Percentage(45),
            ratatui::layout::Constraint::Percentage(30),
        ],
    )
    .header(
        Row::new(vec!["ID", "Path", "Devices"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("Folders (a: add, d: delete)"))
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(app.folder_selected));
    f.render_stateful_widget(table, area, &mut state);
}
