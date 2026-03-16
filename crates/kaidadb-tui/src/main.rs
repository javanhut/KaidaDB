mod app;
mod client;
mod ui;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::Duration;

use app::{App, InputMode};

#[derive(Parser)]
#[command(name = "kaidadb-tui", version, about = "KaidaDB interactive terminal UI")]
struct Args {
    /// Server gRPC address
    #[arg(short, long, default_value = "http://localhost:50051")]
    addr: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, args.addr).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {e}");
    }

    Ok(())
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    addr: String,
) -> Result<()> {
    let mut app = App::new(addr.clone());

    // Initial connect + load
    app.connect().await;
    app.refresh_media_list().await;
    app.check_health().await;

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            break
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.previous(),
                        KeyCode::Down | KeyCode::Char('j') => app.next(),
                        KeyCode::Home | KeyCode::Char('g') => app.first(),
                        KeyCode::End | KeyCode::Char('G') => app.last(),
                        KeyCode::Enter => app.view_detail(),
                        KeyCode::Char('r') => app.refresh_media_list().await,
                        KeyCode::Char('d') => app.enter_delete_confirm(),
                        KeyCode::Char('s') => app.enter_store_mode(),
                        KeyCode::Char('/') => app.enter_search_mode(),
                        KeyCode::Char('n') => app.search_next(),
                        KeyCode::Tab => app.toggle_panel(),
                        KeyCode::Esc => app.back(),
                        _ => {}
                    },
                    InputMode::Search => match key.code {
                        KeyCode::Enter => {
                            app.execute_search();
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Esc => {
                            app.search_input.clear();
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char(c) => app.search_input.push(c),
                        KeyCode::Backspace => {
                            app.search_input.pop();
                        }
                        _ => {}
                    },
                    InputMode::StoreKey => match key.code {
                        KeyCode::Enter | KeyCode::Tab => app.advance_to_browser(),
                        KeyCode::Esc => {
                            app.store_key_input.clear();
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Left => app.store_key_move_left(),
                        KeyCode::Right => app.store_key_move_right(),
                        KeyCode::Home => app.store_key_home(),
                        KeyCode::End => app.store_key_end(),
                        KeyCode::Backspace => app.store_key_backspace(),
                        KeyCode::Delete => app.store_key_delete(),
                        KeyCode::Char(c) => app.store_key_insert_char(c),
                        _ => {}
                    },
                    InputMode::FileBrowser => match key.code {
                        KeyCode::Esc => {
                            app.store_key_input.clear();
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Tab => {
                            // Tab back to key input
                            app.input_mode = InputMode::StoreKey;
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.browser_previous(),
                        KeyCode::Down | KeyCode::Char('j') => app.browser_next(),
                        KeyCode::Home | KeyCode::Char('g') => app.browser_first(),
                        KeyCode::End | KeyCode::Char('G') => app.browser_last(),
                        KeyCode::PageDown => app.browser_page_down(20),
                        KeyCode::PageUp => app.browser_page_up(20),
                        KeyCode::Left | KeyCode::Backspace => app.browser_go_up(),
                        KeyCode::Right => {
                            // Right arrow enters dirs, same as Enter for dirs
                            if let Some(entry) = app.browser_selected_entry() {
                                if entry.is_dir {
                                    app.browser_enter();
                                }
                            }
                        }
                        KeyCode::Enter => {
                            if app.browser_selected_is_file() {
                                let entry = app.browser_entries[app.browser_selected].clone();
                                app.suggest_key_from_path(&entry.path);
                                if app.store_key_input.is_empty() {
                                    // Need a key first
                                    app.status_message =
                                        "Enter a key first (Tab to go back)".into();
                                    app.input_mode = InputMode::StoreKey;
                                } else {
                                    app.execute_store_file(&entry.path).await;
                                    app.input_mode = InputMode::Normal;
                                }
                            } else {
                                app.browser_enter();
                            }
                        }
                        KeyCode::Char('.') => {
                            // Toggle showing hidden files — just reload for now
                            app.load_browser_dir();
                        }
                        _ => {}
                    },
                    InputMode::DeleteConfirm => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            app.execute_delete().await;
                            app.input_mode = InputMode::Normal;
                        }
                        _ => {
                            app.input_mode = InputMode::Normal;
                        }
                    },
                    InputMode::Detail => match key.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char('d') => app.enter_delete_confirm(),
                        _ => {}
                    },
                }
            }
        }
    }

    Ok(())
}
