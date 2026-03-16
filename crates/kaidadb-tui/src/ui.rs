use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::app::{App, InputMode, Panel};

pub fn draw(f: &mut Frame, app: &App) {
    match app.input_mode {
        InputMode::Detail => draw_detail_view(f, app),
        InputMode::StoreKey | InputMode::StorePath => {
            draw_main_layout(f, app);
            draw_store_dialog(f, app);
        }
        InputMode::DeleteConfirm => {
            draw_main_layout(f, app);
            draw_delete_confirm(f, app);
        }
        InputMode::Search => {
            draw_main_layout(f, app);
            draw_search_bar(f, app);
        }
        _ => draw_main_layout(f, app),
    }
}

fn draw_main_layout(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(5),   // body
            Constraint::Length(3), // status bar
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_body(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let connected_indicator = if app.connected {
        Span::styled(" CONNECTED ", Style::default().fg(Color::Black).bg(Color::Green))
    } else {
        Span::styled(" DISCONNECTED ", Style::default().fg(Color::White).bg(Color::Red))
    };

    let title = Line::from(vec![
        Span::styled(
            " KaidaDB ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        connected_indicator,
        Span::raw("  "),
        Span::styled(
            format!("v{}", app.server_version),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} items", app.filtered_items.len()),
            Style::default().fg(Color::Yellow),
        ),
        if !app.search_query.is_empty() {
            Span::styled(
                format!("  filter: \"{}\"", app.search_query),
                Style::default().fg(Color::Magenta),
            )
        } else {
            Span::raw("")
        },
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let header = Paragraph::new(title).block(block);
    f.render_widget(header, area);
}

fn draw_body(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_media_list(f, app, chunks[0]);
    draw_preview_panel(f, app, chunks[1]);
}

fn draw_media_list(f: &mut Frame, app: &App, area: Rect) {
    let highlight_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let items: Vec<ListItem> = app
        .filtered_items
        .iter()
        .enumerate()
        .map(|(i, &idx)| {
            let item = &app.items[idx];
            let size_str = format_size(item.total_size);
            let line = Line::from(vec![
                Span::styled(
                    format!("{:40}", truncate(&item.key, 40)),
                    if i == app.selected {
                        highlight_style
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    format!(" {:>8}", size_str),
                    if i == app.selected {
                        highlight_style
                    } else {
                        Style::default().fg(Color::Yellow)
                    },
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let border_color = if app.active_panel == Panel::List {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let list = List::new(items).block(
        Block::default()
            .title(" Media ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color)),
    );

    f.render_widget(list, area);
}

fn draw_preview_panel(f: &mut Frame, app: &App, area: Rect) {
    let border_color = if app.active_panel == Panel::Detail {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(" Details ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    if let Some(item) = app.selected_item() {
        let rows = vec![
            Row::new(vec![
                Span::styled("Key", Style::default().fg(Color::DarkGray)),
                Span::styled(item.key.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]),
            Row::new(vec![
                Span::styled("Size", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} ({})", format_size(item.total_size), format_bytes(item.total_size)),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Row::new(vec![
                Span::styled("Type", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    item.content_type.clone(),
                    Style::default().fg(Color::Green),
                ),
            ]),
            Row::new(vec![
                Span::styled("Chunks", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    item.chunk_count.to_string(),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Row::new(vec![
                Span::styled("Checksum", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    truncate(&item.checksum, 32).to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Row::new(vec![
                Span::styled("Created", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format_timestamp(item.created_at),
                    Style::default().fg(Color::White),
                ),
            ]),
        ];

        let metadata_rows: Vec<Row> = item
            .metadata
            .iter()
            .map(|(k, v)| {
                Row::new(vec![
                    Span::styled(k.clone(), Style::default().fg(Color::Magenta)),
                    Span::styled(v.clone(), Style::default().fg(Color::White)),
                ])
            })
            .collect();

        let all_rows: Vec<Row> = rows.into_iter().chain(metadata_rows).collect();

        let table = Table::new(
            all_rows,
            [Constraint::Length(12), Constraint::Min(20)],
        )
        .block(block);

        f.render_widget(table, area);
    } else {
        let empty = Paragraph::new(Span::styled(
            "No item selected",
            Style::default().fg(Color::DarkGray),
        ))
        .block(block);
        f.render_widget(empty, area);
    }
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let keybinds = match app.input_mode {
        InputMode::Normal => {
            " q Quit │ j/k Navigate │ Enter Detail │ s Store │ d Delete │ / Search │ r Refresh "
        }
        InputMode::Detail => " Esc Back │ d Delete ",
        _ => "",
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let status = Paragraph::new(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            &app.status_message,
            Style::default().fg(Color::White),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    let keys = Paragraph::new(Span::styled(
        keybinds,
        Style::default().fg(Color::DarkGray),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(status, chunks[0]);
    f.render_widget(keys, chunks[1]);
}

fn draw_detail_view(f: &mut Frame, app: &App) {
    let area = f.area();

    let item = match &app.detail_item {
        Some(i) => i,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // title
            Constraint::Min(10),   // content
            Constraint::Length(3), // status
        ])
        .split(area);

    // Title
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " Media Detail ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(&item.key, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(title, chunks[0]);

    // Content
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Key:          ", Style::default().fg(Color::DarkGray)),
            Span::styled(&item.key, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  Size:         ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} ({})", format_size(item.total_size), format_bytes(item.total_size)),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Content-Type: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&item.content_type, Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("  Chunks:       ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                item.chunk_count.to_string(),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Checksum:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(&item.checksum, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Created:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_timestamp(item.created_at),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Updated:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_timestamp(item.updated_at),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    if !item.metadata.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Custom Metadata:",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )));
        for (k, v) in &item.metadata {
            lines.push(Line::from(vec![
                Span::styled(format!("    {k}: "), Style::default().fg(Color::Magenta)),
                Span::styled(v, Style::default().fg(Color::White)),
            ]));
        }
    }

    let content = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(content, chunks[1]);

    // Status
    let status = Paragraph::new(Span::styled(
        " Esc Back │ d Delete ",
        Style::default().fg(Color::DarkGray),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(status, chunks[2]);
}

fn draw_store_dialog(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 9, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Store Media ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                if app.input_mode == InputMode::StoreKey { "▸ " } else { "  " },
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("Key:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &app.store_key_input,
                Style::default().fg(if app.input_mode == InputMode::StoreKey {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ),
            if app.input_mode == InputMode::StoreKey {
                Span::styled("█", Style::default().fg(Color::Yellow))
            } else {
                Span::raw("")
            },
        ]),
        Line::from(vec![
            Span::styled(
                if app.input_mode == InputMode::StorePath { "▸ " } else { "  " },
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("Path: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &app.store_path_input,
                Style::default().fg(if app.input_mode == InputMode::StorePath {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ),
            if app.input_mode == InputMode::StorePath {
                Span::styled("█", Style::default().fg(Color::Yellow))
            } else {
                Span::raw("")
            },
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Enter: next/confirm │ Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let dialog = Paragraph::new(lines);
    f.render_widget(dialog, inner);
}

fn draw_delete_confirm(f: &mut Frame, app: &App) {
    let area = centered_rect(50, 7, f.area());
    f.render_widget(Clear, area);

    let key = app
        .selected_item()
        .map(|i| i.key.as_str())
        .unwrap_or("?");

    let block = Block::default()
        .title(" Confirm Delete ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Delete "),
            Span::styled(key, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw("?"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  y Yes │ any other key: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let dialog = Paragraph::new(lines);
    f.render_widget(dialog, inner);
}

fn draw_search_bar(f: &mut Frame, app: &App) {
    let area = centered_rect(50, 3, f.area());
    f.render_widget(Clear, area);

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" / ", Style::default().fg(Color::Magenta)),
        Span::styled(&app.search_input, Style::default().fg(Color::White)),
        Span::styled("█", Style::default().fg(Color::Magenta)),
    ]))
    .block(
        Block::default()
            .title(" Search ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta)),
    );

    f.render_widget(input, area);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_bytes(bytes: u64) -> String {
    format!("{bytes} bytes")
}

fn format_timestamp(ts: i64) -> String {
    if ts == 0 {
        return "—".into();
    }
    // Simple UTC formatting without pulling in chrono
    let secs_per_day: i64 = 86400;
    let days = ts / secs_per_day;
    let time_of_day = ts % secs_per_day;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since epoch to Y-M-D (simplified)
    let mut y = 1970i64;
    let mut remaining_days = days;

    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining_days < md {
            m = i;
            break;
        }
        remaining_days -= md;
    }

    format!(
        "{y:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        m + 1,
        remaining_days + 1,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
