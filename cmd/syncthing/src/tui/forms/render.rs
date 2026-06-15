//! 统一表单渲染 — 将 add/edit 对的渲染逻辑合并为 2 个函数（device / folder）。

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::theme::Theme;

use super::FormState;

/// 渲染 device 表单（add + edit 共用）。
/// edit 模式下 field[0] 为只读，`form.fields[0].editable` 控制渲染样式。
pub fn draw_device_form(f: &mut Frame, form: &FormState, theme: &Theme) {
    // 半透明背景
    let dim = Block::default().style(Style::default().bg(Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());

    let area = super::super::widgets::centered_popup(form.width, form.height, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused)
        .title(Span::styled(
            format!(" {} ", form.title),
            theme.style_header,
        ));

    let field_count = form.field_count();
    let mut constraints: Vec<Constraint> =
        form.fields.iter().map(|_| Constraint::Length(3)).collect();
    let hints_present = form.fields.iter().any(|f| {
        f.hint.is_some()
            && form.focus
                == form
                    .fields
                    .iter()
                    .position(|g| g.label == f.label)
                    .unwrap_or(usize::MAX)
    });
    if hints_present {
        constraints.push(Constraint::Length(1));
    }
    if form.error.is_some() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(2)); // footer

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(area);

    for (i, field) in form.fields.iter().enumerate() {
        let focused = form.focus == i;
        let display = if focused && field.value.is_empty() && field.editable {
            format!("{}: ▎", field.label)
        } else {
            format!("{}: {}", field.label, field.value)
        };

        let style = if field.editable {
            if focused {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_secondary)
            }
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let para = Paragraph::new(display).style(style).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if focused {
                    theme.border_focused
                } else {
                    theme.border
                }),
        );
        f.render_widget(para, chunks[i]);
    }

    let mut next_idx = field_count;
    if hints_present {
        let hint_text = form
            .fields
            .get(form.focus)
            .and_then(|f| f.hint)
            .unwrap_or("");
        let hint = Paragraph::new(Span::styled(hint_text, theme.style_idle));
        f.render_widget(hint, chunks[next_idx]);
        next_idx += 1;
    }
    if let Some(ref error) = form.error {
        let error_line = Paragraph::new(Span::styled(format!("⚠ {}", error), theme.style_error));
        f.render_widget(error_line, chunks[next_idx]);
        next_idx += 1;
    }

    // Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab", theme.style_header),
        Span::styled(" next  ", theme.style_idle),
        Span::styled("Enter", theme.style_header),
        Span::styled(" save  ", theme.style_idle),
        Span::styled("Esc", theme.style_header),
        Span::styled(" cancel", theme.style_idle),
    ]));
    f.render_widget(footer, chunks[next_idx]);
    f.render_widget(block, area);
}

/// 渲染 folder 表单（add + edit 共用），含 device 多选列表。
pub fn draw_folder_form(
    f: &mut Frame,
    form: &FormState,
    theme: &Theme,
    devices: &[syncthing_core::types::Device],
    device_selection: &[bool],
    device_selected: usize,
) {
    let dim = Block::default().style(Style::default().bg(Color::Rgb(20, 20, 25)));
    f.render_widget(dim, f.area());

    let area = super::super::widgets::centered_popup(form.width, form.height, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused)
        .title(Span::styled(
            format!(" {} ", form.title),
            theme.style_header,
        ));

    let field_count = form.field_count();
    let mut constraints: Vec<Constraint> =
        form.fields.iter().map(|_| Constraint::Length(3)).collect();
    constraints.push(Constraint::Length(1)); // hint
    if form.error.is_some() {
        constraints.push(Constraint::Length(1)); // error
    }
    constraints.push(Constraint::Min(4)); // device list
    constraints.push(Constraint::Length(2)); // footer

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(area);

    // Text fields
    for (i, field) in form.fields.iter().enumerate() {
        let focused = form.focus == i;
        let display = if focused && field.value.is_empty() && field.editable {
            format!("{}: ▎", field.label)
        } else {
            format!("{}: {}", field.label, field.value)
        };

        let style = if field.editable {
            if focused {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_secondary)
            }
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let para = Paragraph::new(display).style(style).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(if focused {
                    theme.border_focused
                } else {
                    theme.border
                }),
        );
        f.render_widget(para, chunks[i]);
    }

    // Hint
    let hint_text = form
        .fields
        .get(form.focus)
        .and_then(|f| f.hint)
        .unwrap_or("");
    let hint = Paragraph::new(Span::styled(hint_text, theme.style_idle));
    f.render_widget(hint, chunks[field_count]);

    let mut next_idx = field_count + 1;
    if let Some(ref error) = form.error {
        let error_line = Paragraph::new(Span::styled(format!("⚠ {}", error), theme.style_error));
        f.render_widget(error_line, chunks[next_idx]);
        next_idx += 1;
    }

    // Device selection list
    let list_focus = form.is_on_list();
    let items: Vec<ListItem> = devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let checked = device_selection.get(i).copied().unwrap_or(false);
            let marker = if checked { "[x]" } else { "[ ]" };
            let is_highlighted = list_focus && device_selected == i;
            let style = if is_highlighted {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_secondary)
            };
            let name = d.name.as_deref().unwrap_or("Unnamed");
            ListItem::new(Line::from(format!("{} {} — {}", marker, name, d.id))).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if list_focus {
                theme.border_focused
            } else {
                theme.border
            })
            .title(Span::styled(" Share with ", theme.style_header)),
    );
    f.render_widget(list, chunks[next_idx]);
    next_idx += 1;

    // Footer
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Tab", theme.style_header),
        Span::styled(" next  ", theme.style_idle),
        Span::styled("Space", theme.style_header),
        Span::styled(" toggle  ", theme.style_idle),
        Span::styled("Enter", theme.style_header),
        Span::styled(" save  ", theme.style_idle),
        Span::styled("Esc", theme.style_header),
        Span::styled(" cancel", theme.style_idle),
    ]));
    f.render_widget(footer, chunks[next_idx]);
    f.render_widget(block, area);
}
