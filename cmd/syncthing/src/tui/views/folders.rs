use ratatui::{
    style::Modifier,
    text::Span,
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

use crate::tui::app::App;

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
            Row::new(vec![
                Cell::from(Span::styled(fo.id.clone(), theme.style_header)),
                Cell::from(Span::styled(fo.path.clone(), theme.style_idle)),
                Cell::from(Span::styled(devs, theme.style_idle)),
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
        Row::new(vec![
            Cell::from(Span::styled("ID", theme.style_header)),
            Cell::from(Span::styled("Path", theme.style_header)),
            Cell::from(Span::styled("Devices", theme.style_header)),
        ])
        .style(Modifier::BOLD),
    )
    .block(Block::default().borders(Borders::ALL).title("Folders (a: add, d: delete)"))
    .row_highlight_style(theme.style_header.add_modifier(Modifier::REVERSED));

    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(app.folder_selected));
    f.render_stateful_widget(table, area, &mut state);
}
