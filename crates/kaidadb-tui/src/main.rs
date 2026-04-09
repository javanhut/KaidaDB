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
                        KeyCode::Char('q') => {
                            if !app.browse_prefix.is_empty() {
                                app.browse_up();
                            } else {
                                break;
                            }
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            break
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.previous(),
                        KeyCode::Down | KeyCode::Char('j') => app.next(),
                        KeyCode::Home | KeyCode::Char('g') => app.first(),
                        KeyCode::End | KeyCode::Char('G') => app.last(),
                        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app.view_detail(),
                        KeyCode::Left | KeyCode::Backspace => app.browse_up(),
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
                    InputMode::PathBrowser => match key.code {
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.path_previous(),
                        KeyCode::Down | KeyCode::Char('j') => app.path_next(),
                        KeyCode::Home | KeyCode::Char('g') => app.path_first(),
                        KeyCode::End | KeyCode::Char('G') => app.path_last(),
                        KeyCode::Enter | KeyCode::Right => {
                            if let Some(entry) = app.path_selected_entry() {
                                if entry.is_dir {
                                    app.path_enter();
                                }
                            }
                        }
                        KeyCode::Left | KeyCode::Backspace => app.path_go_up(),
                        KeyCode::Char('n') => app.enter_new_dir_mode(),
                        KeyCode::Tab | KeyCode::Char('f') => {
                            app.advance_to_file_browser();
                        }
                        _ => {}
                    },
                    InputMode::NewDirInput => match key.code {
                        KeyCode::Enter => app.confirm_new_dir(),
                        KeyCode::Esc => {
                            app.input_mode = InputMode::PathBrowser;
                        }
                        KeyCode::Left => app.new_dir_move_left(),
                        KeyCode::Right => app.new_dir_move_right(),
                        KeyCode::Backspace => app.new_dir_backspace(),
                        KeyCode::Delete => app.new_dir_delete(),
                        KeyCode::Char(c) => app.new_dir_insert_char(c),
                        _ => {}
                    },
                    InputMode::StoreKey => match key.code {
                        KeyCode::Enter => {
                            if !app.store_key_input.is_empty() {
                                if let Some(file_path) = app.selected_file_path.clone() {
                                    app.execute_store_file(&file_path).await;
                                    app.input_mode = InputMode::Normal;
                                }
                            }
                        }
                        KeyCode::Esc => {
                            app.store_key_input.clear();
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Tab => app.advance_to_browser(),
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
                            app.selected_file_path = None;
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Tab => {
                            app.back_to_path_browser();
                        }
                        KeyCode::Up | KeyCode::Char('k') => app.browser_previous(),
                        KeyCode::Down | KeyCode::Char('j') => app.browser_next(),
                        KeyCode::Home | KeyCode::Char('g') => app.browser_first(),
                        KeyCode::End | KeyCode::Char('G') => app.browser_last(),
                        KeyCode::PageDown => app.browser_page_down(20),
                        KeyCode::PageUp => app.browser_page_up(20),
                        KeyCode::Left | KeyCode::Backspace => app.browser_go_up(),
                        KeyCode::Right => {
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
                                app.selected_file_path = Some(entry.path);
                                app.input_mode = InputMode::StoreKey;
                            } else {
                                app.browser_enter();
                            }
                        }
                        KeyCode::Char('.') => {
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
