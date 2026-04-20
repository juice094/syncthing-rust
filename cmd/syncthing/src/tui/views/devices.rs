use ratatui::{
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, HighlightSpacing, List, ListItem},
    Frame,
};

use crate::tui::app::App;

pub fn draw(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let theme = &app.theme;
    let items: Vec<ListItem> = app
        .config
        .devices
        .iter()
        .map(|d| {
            let id_short = d.id.to_string();
            let addr = d
                .addresses
                .first()
                .map(|a| a.as_str().to_string())
                .unwrap_or_else(|| "dynamic".to_string());
            let connected = app.connected_devices.contains(&d.id);
            let status = if connected { "● Online" } else { "○ Offline" };
            let line = Line::from(vec![
                Span::raw(format!("{} ", d.name.as_deref().unwrap_or("Unnamed"))),
                Span::styled(id_short, theme.style_idle),
                Span::raw(" "),
                Span::styled(addr, theme.style_idle),
                Span::raw(" "),
                Span::styled(status, if connected { theme.style_online } else { theme.style_offline }),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Devices (a: add, d: delete)"))
        .highlight_style(theme.style_header.add_modifier(Modifier::REVERSED))
        .highlight_spacing(HighlightSpacing::Always)
        .scroll_padding(1);

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(app.device_selected));
    f.render_stateful_widget(list, area, &mut state);
}
