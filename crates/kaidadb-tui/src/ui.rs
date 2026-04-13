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
        InputMode::PathBrowser | InputMode::NewDirInput => draw_path_browser_view(f, app),
        InputMode::StoreKey | InputMode::FileBrowser => draw_store_view(f, app),
        InputMode::DeleteConfirm => {
            draw_main_layout(f, app);
            draw_delete_confirm(f, app);
        }
        InputMode::RenameInput => {
            draw_main_layout(f, app);
            draw_rename_dialog(f, app);
        }
        InputMode::MkdirInput => {
            draw_main_layout(f, app);
            draw_mkdir_dialog(f, app);
        }
        InputMode::Search => {
            draw_main_layout(f, app);
            draw_search_bar(f, app);
        }
        _ => draw_main_layout(f, app),
    }
}

// ── Main Layout ──────────────────────────────────────────────────────

fn draw_main_layout(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_body(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let connected_indicator = if app.connected {
        Span::styled(
            " CONNECTED ",
            Style::default().fg(Color::Black).bg(Color::Green),
        )
    } else {
        Span::styled(
            " DISCONNECTED ",
            Style::default().fg(Color::White).bg(Color::Red),
        )
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
            format!("{} items", app.items.len()),
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

    f.render_widget(Paragraph::new(title).block(block), area);
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
    let hl = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let title = if app.browse_prefix.is_empty() {
        " Media / ".to_string()
    } else {
        format!(" /{} ", app.browse_prefix)
    };

    let items: Vec<ListItem> = app
        .browse_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let is_selected = i == app.selected;

            if entry.is_dir {
                let icon_style = if is_selected {
                    hl
                } else {
                    Style::default().fg(Color::Cyan)
                };
                let name_style = if is_selected {
                    hl
                } else {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                };
                let count_style = if is_selected {
                    hl
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(" ", icon_style),
                    Span::styled(format!(" {}/", entry.name), name_style),
                    Span::styled(
                        format!("  ({} items)", entry.item_count),
                        count_style,
                    ),
                ]))
            } else {
                let icon_style = if is_selected {
                    hl
                } else {
                    Style::default().fg(file_color(&entry.name))
                };
                let name_style = if is_selected {
                    hl
                } else {
                    Style::default().fg(Color::White)
                };
                let size_style = if is_selected {
                    hl
                } else {
                    Style::default().fg(Color::Yellow)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(" ", icon_style),
                    Span::styled(format!(" {}", entry.name), name_style),
                    Span::styled(format!("  {:>8}", format_size(entry.size)), size_style),
                ]))
            }
        })
        .collect();

    let border = if app.active_panel == Panel::List {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let list = List::new(items).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border)),
    );
    f.render_widget(list, area);
}

fn draw_preview_panel(f: &mut Frame, app: &App, area: Rect) {
    let border = if app.active_panel == Panel::Detail {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let block = Block::default()
        .title(" Details ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border));

    if let Some(item) = app.selected_item() {
        let size_str = format!(
            "{} ({})",
            format_size(item.total_size),
            format_bytes(item.total_size)
        );
        let chunks_str = item.chunk_count.to_string();
        let created_str = format_timestamp(item.created_at);
        let rows = vec![
            detail_row("Key", &item.key, Color::White, true),
            detail_row("Size", &size_str, Color::Yellow, false),
            detail_row("Type", &item.content_type, Color::Green, false),
            detail_row("Chunks", &chunks_str, Color::Cyan, false),
            detail_row("Checksum", truncate(&item.checksum, 32), Color::DarkGray, false),
            detail_row("Created", &created_str, Color::White, false),
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
        let table = Table::new(all_rows, [Constraint::Length(12), Constraint::Min(20)]).block(block);
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

fn detail_row<'a>(label: &'a str, value: &'a str, color: Color, bold: bool) -> Row<'a> {
    let mut style = Style::default().fg(color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Row::new(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::styled(value, style),
    ])
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let keybinds = match app.input_mode {
        InputMode::Normal => {
            if app.browse_prefix.is_empty() {
                " q Quit │ j/k Nav │ Enter Open │ s Store │ d Del │ m Rename │ M Mkdir │ / Search │ r Refresh "
            } else {
                " q/Bksp Back │ j/k Nav │ Enter Open │ s Store │ d Del │ m Rename │ M Mkdir │ / Search "
            }
        }
        InputMode::Detail => " Esc Back │ d Delete │ m Rename ",
        _ => "",
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let status = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(&app.status_message, Style::default().fg(Color::White)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    let keys = Paragraph::new(Span::styled(keybinds, Style::default().fg(Color::DarkGray))).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(status, chunks[0]);
    f.render_widget(keys, chunks[1]);
}

// ── Detail View ──────────────────────────────────────────────────────

fn draw_detail_view(f: &mut Frame, app: &App) {
    let item = match &app.detail_item {
        Some(i) => i,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " Media Detail ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            &item.key,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(title, chunks[0]);

    let size_str = format!(
        "{} ({})",
        format_size(item.total_size),
        format_bytes(item.total_size)
    );
    let chunks_str = item.chunk_count.to_string();
    let created_str = format_timestamp(item.created_at);
    let updated_str = format_timestamp(item.updated_at);

    let mut lines = vec![
        Line::from(""),
        labeled_line("  Key:          ", &item.key, Color::White, true),
        labeled_line("  Size:         ", &size_str, Color::Yellow, false),
        labeled_line("  Content-Type: ", &item.content_type, Color::Green, false),
        labeled_line("  Chunks:       ", &chunks_str, Color::Cyan, false),
        labeled_line("  Checksum:     ", &item.checksum, Color::White, false),
        labeled_line("  Created:      ", &created_str, Color::White, false),
        labeled_line("  Updated:      ", &updated_str, Color::White, false),
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
                Span::styled(v.as_str(), Style::default().fg(Color::White)),
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

// ── Path Browser View (navigate KaidaDB virtual directory tree) ──────

fn draw_path_browser_view(f: &mut Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Length(3), // current path
            Constraint::Min(8),   // path entries
            Constraint::Length(3), // status/keybinds
        ])
        .split(area);

    // Title bar
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " Store Media ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            "Navigate to a destination path, then pick a file",
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(title, chunks[0]);

    // Current path display
    let path_display = if app.path_prefix.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", app.path_prefix)
    };
    let path_line = Paragraph::new(Line::from(vec![
        Span::styled(
            "  Path: ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &path_display,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(path_line, chunks[1]);

    // Path entries list
    let is_active = app.input_mode == InputMode::PathBrowser;
    let border_color = if is_active {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(if app.path_entries.is_empty() {
            " Empty - press 'n' to create a directory or Tab to pick a file "
        } else {
            " Directories & Items "
        })
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(chunks[2]);
    f.render_widget(block, chunks[2]);

    if inner.height > 0 {
        let visible_rows = inner.height as usize;

        let scroll_offset = if app.path_selected < app.path_scroll_offset {
            app.path_selected
        } else if app.path_selected >= app.path_scroll_offset + visible_rows {
            app.path_selected - visible_rows + 1
        } else {
            app.path_scroll_offset
        };

        let hl = Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD);

        let items: Vec<ListItem> = app
            .path_entries
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_rows)
            .map(|(i, entry)| {
                let is_selected = i == app.path_selected;

                if entry.is_dir {
                    let name_style = if is_selected {
                        hl
                    } else {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    };
                    let count_style = if is_selected {
                        hl
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    let icon_style = if is_selected {
                        hl
                    } else {
                        Style::default().fg(Color::Cyan)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled("  ", icon_style),
                        Span::styled(format!("{}/", entry.name), name_style),
                        Span::styled(
                            format!("  ({} items)", entry.item_count),
                            count_style,
                        ),
                    ]))
                } else {
                    let name_style = if is_selected {
                        hl
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let icon_style = if is_selected {
                        hl
                    } else {
                        Style::default().fg(file_color(&entry.name))
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled("  ", icon_style),
                        Span::styled(&entry.name, name_style),
                    ]))
                }
            })
            .collect();

        let list = List::new(items);
        f.render_widget(list, inner);
    }

    // New directory input overlay
    if app.input_mode == InputMode::NewDirInput {
        let dialog_width = 50u16.min(area.width.saturating_sub(4));
        let dialog_area = centered_rect(dialog_width, 5, area);
        f.render_widget(Clear, dialog_area);

        let dir_text = &app.new_dir_input;
        let cursor_pos = app.new_dir_cursor;
        let (before, after) = dir_text.split_at(cursor_pos.min(dir_text.len()));
        let (cursor_char, rest) = if after.is_empty() {
            (" ", "")
        } else {
            after.split_at(
                after
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| i)
                    .unwrap_or(after.len()),
            )
        };

        let lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Name: ", Style::default().fg(Color::Yellow)),
                Span::styled(before, Style::default().fg(Color::White)),
                Span::styled(
                    cursor_char,
                    Style::default().fg(Color::Black).bg(Color::Yellow),
                ),
                Span::styled(rest, Style::default().fg(Color::White)),
            ]),
            Line::from(Span::styled(
                "  Enter: confirm │ Esc: cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let dialog = Paragraph::new(lines).block(
            Block::default()
                .title(" New Directory ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        );
        f.render_widget(dialog, dialog_area);
    }

    // Keybinds
    let help = match app.input_mode {
        InputMode::PathBrowser => {
            " Enter/Right: open dir │ Left: parent │ n: new dir │ Tab/f: pick file │ Esc: cancel "
        }
        InputMode::NewDirInput => " Enter: confirm │ Esc: cancel ",
        _ => "",
    };
    let status = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(&app.status_message, Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled(help, Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(status, chunks[3]);
}

// ── Store View (full screen with file browser) ───────────────────────

fn draw_store_view(f: &mut Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Length(5), // key input
            Constraint::Min(8),   // file browser
            Constraint::Length(3), // status/keybinds
        ])
        .split(area);

    // Title bar
    let subtitle = match app.input_mode {
        InputMode::FileBrowser => "Select a file to store",
        InputMode::StoreKey => "Review the key and press Enter to store",
        _ => "Select a file to store",
    };
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " Store Media ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Path: {}  -  {}", if app.path_prefix.is_empty() { "/" } else { &app.path_prefix }, subtitle),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(title, chunks[0]);

    // Key input area
    draw_key_input(f, app, chunks[1]);

    // File browser
    draw_file_browser(f, app, chunks[2]);

    // Keybinds
    let help = match app.input_mode {
        InputMode::StoreKey => {
            " Enter: confirm store │ Tab: back to file browser │ Left/Right: move cursor │ Esc: cancel "
        }
        InputMode::FileBrowser => {
            " Enter: select file/open dir │ Left: parent dir │ Tab: back to path │ Esc: cancel "
        }
        _ => "",
    };
    let status = Paragraph::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(&app.status_message, Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled(help, Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(status, chunks[3]);
}

fn draw_key_input(f: &mut Frame, app: &App, area: Rect) {
    let is_active = app.input_mode == InputMode::StoreKey;
    let border_color = if is_active {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let label_style = if is_active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Build the key text with a visible cursor
    let key_text = &app.store_key_input;
    let cursor_pos = app.store_key_cursor;

    let (before, after) = key_text.split_at(cursor_pos.min(key_text.len()));
    let (cursor_char, rest) = if after.is_empty() {
        (" ", "")
    } else {
        after.split_at(after.char_indices().nth(1).map(|(i, _)| i).unwrap_or(after.len()))
    };

    let text_style = Style::default().fg(Color::White);
    let cursor_style = if is_active {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        text_style
    };

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Key: ", label_style),
            Span::styled(before, text_style),
            Span::styled(cursor_char, cursor_style),
            Span::styled(rest, text_style),
        ]),
        Line::from(vec![
            Span::styled(
                "        e.g. tv/breaking-bad/s01/e01-pilot",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    let block = Block::default()
        .title(if is_active { " Key (editing) " } else { " Key " })
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_file_browser(f: &mut Frame, app: &App, area: Rect) {
    let is_active = app.input_mode == InputMode::FileBrowser;
    let border_color = if is_active {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(format!(" {} ", app.browser_dir.display()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let visible_rows = inner.height as usize;

    // Compute scroll offset
    let scroll_offset = if app.browser_selected < app.browser_scroll_offset {
        app.browser_selected
    } else if app.browser_selected >= app.browser_scroll_offset + visible_rows {
        app.browser_selected - visible_rows + 1
    } else {
        app.browser_scroll_offset
    };

    let hl = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let items: Vec<ListItem> = app
        .browser_entries
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
        .map(|(i, entry)| {
            let is_selected = i == app.browser_selected;

            let icon = if entry.is_dir { " " } else { " " };
            let icon_color = if entry.is_dir {
                Color::Cyan
            } else {
                file_color(&entry.name)
            };

            let name_style = if is_selected {
                hl
            } else if entry.is_dir {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let size_str = if entry.is_dir {
                "    DIR".to_string()
            } else {
                format!("{:>7}", format_size(entry.size))
            };

            let size_style = if is_selected {
                hl
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let icon_style = if is_selected { hl } else { Style::default().fg(icon_color) };

            ListItem::new(Line::from(vec![
                Span::styled(icon, icon_style),
                Span::styled(format!("{:<}", &entry.name), name_style),
                Span::styled(format!("  {}", size_str), size_style),
            ]))
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}

// ── Delete Confirm ───────────────────────────────────────────────────

fn draw_delete_confirm(f: &mut Frame, app: &App) {
    let key = app
        .selected_item()
        .map(|i| i.key.as_str())
        .unwrap_or("?");

    // Size the dialog to fit the key
    let width = (key.len() as u16 + 20).max(40).min(f.area().width - 4);
    let area = centered_rect(width, 7, f.area());
    f.render_widget(Clear, area);

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
            Span::styled(
                key,
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ?"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  y Yes │ any other key: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ── Rename Dialog ───────────────────────────────────────────────────

fn draw_rename_dialog(f: &mut Frame, app: &App) {
    let width = (app.rename_input.len() as u16 + 20).max(50).min(f.area().width - 4);
    let area = centered_rect(width, 7, f.area());
    f.render_widget(Clear, area);

    let text = &app.rename_input;
    let cursor_pos = app.rename_cursor;
    let (before, after) = text.split_at(cursor_pos.min(text.len()));
    let (cursor_char, rest) = if after.is_empty() {
        (" ", "")
    } else {
        after.split_at(
            after
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(after.len()),
        )
    };

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  New key: ", Style::default().fg(Color::Yellow)),
            Span::styled(before, Style::default().fg(Color::White)),
            Span::styled(
                cursor_char,
                Style::default().fg(Color::Black).bg(Color::Yellow),
            ),
            Span::styled(rest, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  From:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &app.rename_original_key,
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(Span::styled(
            "  Enter: confirm │ Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(" Rename / Move ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(dialog, area);
}

// ── Mkdir Dialog ────────────────────────────────────────────────────

fn draw_mkdir_dialog(f: &mut Frame, app: &App) {
    let width = 50u16.min(f.area().width.saturating_sub(4));
    let area = centered_rect(width, 7, f.area());
    f.render_widget(Clear, area);

    let text = &app.mkdir_input;
    let cursor_pos = app.mkdir_cursor;
    let (before, after) = text.split_at(cursor_pos.min(text.len()));
    let (cursor_char, rest) = if after.is_empty() {
        (" ", "")
    } else {
        after.split_at(
            after
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(after.len()),
        )
    };

    let parent = if app.browse_prefix.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", app.browse_prefix)
    };

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Name: ", Style::default().fg(Color::Cyan)),
            Span::styled(before, Style::default().fg(Color::White)),
            Span::styled(
                cursor_char,
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::styled(rest, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  In:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(&parent, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(Span::styled(
            "  Enter: create │ Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let dialog = Paragraph::new(lines).block(
        Block::default()
            .title(" New Directory ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(dialog, area);
}

// ── Search Bar ───────────────────────────────────────────────────────

fn draw_search_bar(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 3, f.area());
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

// ── Helpers ──────────────────────────────────────────────────────────

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

fn labeled_line<'a>(label: &'a str, value: &'a str, color: Color, bold: bool) -> Line<'a> {
    let mut style = Style::default().fg(color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::styled(value, style),
    ])
}

fn file_color(name: &str) -> Color {
    match name.rsplit('.').next() {
        Some("mp4") | Some("mkv") | Some("webm") | Some("avi") | Some("mov") => Color::Green,
        Some("mp3") | Some("flac") | Some("wav") | Some("ogg") | Some("aac") => Color::Magenta,
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("bmp") => {
            Color::Yellow
        }
        Some("txt") | Some("md") | Some("json") | Some("toml") | Some("yaml") | Some("yml") => {
            Color::White
        }
        _ => Color::White,
    }
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
        return "\u{2014}".into(); // em dash
    }
    let secs_per_day: i64 = 86400;
    let days = ts / secs_per_day;
    let time_of_day = ts % secs_per_day;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let mut y = 1970i64;
    let mut remaining_days = days;
    loop {
        let diy = if is_leap(y) { 366 } else { 365 };
        if remaining_days < diy {
            break;
        }
        remaining_days -= diy;
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
