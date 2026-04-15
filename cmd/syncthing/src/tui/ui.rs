use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, HighlightSpacing, List, ListItem, Paragraph, Row, Table,
        Tabs, Wrap,
    },
    Frame,
};

use crate::tui::app::{App, FormState, Popup, Tab};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // 终端过小时显示提示
    if area.width < 40 || area.height < 12 {
        let msg = Paragraph::new("Terminal too small. Please resize to at least 40x12.")
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        f.render_widget(msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    draw_tabs(f, app, chunks[0]);
    draw_main_content(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);

    match app.popup {
        Popup::AddDevice => draw_add_device_popup(f, app),
        Popup::AddFolder => draw_add_folder_popup(f, app),
        Popup::Error(ref msg) => draw_error_popup(f, msg),
        Popup::None => {}
    }
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = [Tab::Overview, Tab::Devices, Tab::Folders, Tab::Logs]
        .iter()
        .map(|t| Line::from(t.title()))
        .collect();

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("syncthing-rust"))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .select(app.tab as usize);

    f.render_widget(tabs, area);
}

fn draw_main_content(f: &mut Frame, app: &mut App, area: Rect) {
    match app.tab {
        Tab::Overview => draw_overview(f, app, area),
        Tab::Devices => draw_devices(f, app, area),
        Tab::Folders => draw_folders(f, app, area),
        Tab::Logs => draw_logs(f, app, area),
    }
}

fn draw_overview(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(0)])
        .split(area);

    let device_id = app
        .config
        .local_device_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let text = Text::from(vec![
        Line::from(vec![
            Span::styled("Device ID: ", Style::default().fg(Color::Cyan)),
            Span::raw(device_id),
        ]),
        Line::from(vec![
            Span::styled("Name:      ", Style::default().fg(Color::Cyan)),
            Span::raw(&app.device_name),
        ]),
        Line::from(vec![
            Span::styled("Listen:    ", Style::default().fg(Color::Cyan)),
            Span::raw(&app.listen),
        ]),
        Line::from(vec![
            Span::styled("Folders:   ", Style::default().fg(Color::Cyan)),
            Span::raw(app.config.folders.len().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Devices:   ", Style::default().fg(Color::Cyan)),
            Span::raw(app.config.devices.len().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Connected: ", Style::default().fg(Color::Cyan)),
            Span::raw(app.connected_devices.len().to_string()),
        ]),
    ]);

    let para = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Overview"))
        .wrap(Wrap { trim: true });
    f.render_widget(para, chunks[0]);

    // Recent logs preview
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

fn draw_devices(f: &mut Frame, app: &App, area: Rect) {
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
            let status = if connected { "[Connected]" } else { "[Offline]" };
            let line = Line::from(vec![
                Span::raw(format!("{} ", d.name.as_deref().unwrap_or("Unnamed"))),
                Span::styled(id_short, Style::default().fg(Color::Gray)),
                Span::raw(" "),
                Span::raw(addr),
                Span::raw(" "),
                Span::styled(status, Style::default().fg(if connected { Color::Green } else { Color::Red })),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Devices (a: add, d: delete)"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_spacing(HighlightSpacing::Always)
        .scroll_padding(1);

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(app.device_selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_folders(f: &mut Frame, app: &App, area: Rect) {
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
            Constraint::Percentage(25),
            Constraint::Percentage(45),
            Constraint::Percentage(30),
        ],
    )
    .header(
        Row::new(vec!["ID", "Path", "Devices"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title("Folders (a: add, d: delete)"))
    .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(app.folder_selected));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_logs(f: &mut Frame, app: &App, area: Rect) {
    let logs: Text = app
        .log_lines
        .iter()
        .map(|l| Line::raw(l.as_str()))
        .collect::<Vec<_>>()
        .into();

    let para = Paragraph::new(logs)
        .block(Block::default().borders(Borders::ALL).title("Logs"))
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let daemon_color = if app.daemon_running { Color::Green } else { Color::Red };
    let status = format!("Daemon: {} | {}", app.daemon_status, app.daemon_running);
    let text = Text::from(vec![
        Line::from(vec![
            Span::styled("F5: Run/Stop  ", Style::default()),
            Span::styled("Tab/←→: Switch  ", Style::default()),
            Span::styled("q: Quit  ", Style::default()),
            Span::styled(status, Style::default().fg(daemon_color)),
        ]),
    ]);
    let para = Paragraph::new(text).alignment(Alignment::Center);
    f.render_widget(para, area);
}

fn draw_add_device_popup(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 40, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Add Device (Tab: next, Enter: save, Esc: cancel)");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let labels = ["Device ID:", "Name:", "Address (e.g. 127.0.0.1:22001):"];
    for (i, label) in labels.iter().enumerate() {
        let text = format!("{} {}", label, app.device_form.fields.get(i).map(|s| s.as_str()).unwrap_or(""));
        let style = if app.device_form.focus == i {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let para = Paragraph::new(text).style(style).block(Block::default().borders(Borders::ALL));
        f.render_widget(para, chunks[i]);
    }

    let hint = Paragraph::new("Type to edit. Backspace to delete.").style(Style::default().fg(Color::Gray));
    f.render_widget(hint, chunks[3]);
    f.render_widget(block, area);
}

fn draw_add_folder_popup(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 50, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Add Folder (Tab: next, Enter: save, Esc: cancel)");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let labels = ["Folder ID:", "Path:"];
    for (i, label) in labels.iter().enumerate() {
        let text = format!("{} {}", label, app.folder_form.fields.get(i).map(|s| s.as_str()).unwrap_or(""));
        let style = if app.folder_form.focus == i {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let para = Paragraph::new(text).style(style).block(Block::default().borders(Borders::ALL));
        f.render_widget(para, chunks[i]);
    }

    // Device selection
    let items: Vec<ListItem> = app
        .config
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let checked = app.folder_device_selection.get(i).copied().unwrap_or(false);
            let marker = if checked { "[x]" } else { "[ ]" };
            let name = d.name.as_deref().unwrap_or("Unnamed");
            let is_highlighted = app.folder_form.focus == app.folder_form.fields.len() && app.folder_device_selected == i;
            let style = if is_highlighted {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("{} {} - {}", marker, name, d.id.to_string())).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Shared with devices (↑↓: move, Space: toggle)"));
    f.render_widget(list, chunks[2]);

    f.render_widget(block, area);
}

fn draw_error_popup(f: &mut Frame, msg: &str) {
    let area = centered_rect(50, 20, f.area());
    f.render_widget(Clear, area);
    let para = Paragraph::new(msg)
        .block(Block::default().borders(Borders::ALL).title("Error (Esc to close)"))
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
