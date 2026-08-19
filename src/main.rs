use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gitloom::app::AppState;
use gitloom::ui;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{error::Error, io};

fn main() -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).map(std::path::PathBuf::from);

    // Create app state
    let mut app_state = AppState::new(path);

    // Run app
    let res = run_app(&mut terminal, &mut app_state);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app_state: &mut AppState,
) -> Result<(), Box<dyn Error>>
where
    B::Error: Into<Box<dyn std::error::Error>> + 'static,
{
    while !app_state.quit {
        terminal
            .draw(|f| ui::render(f, app_state))
            .map_err(|e| e.into())?;

        if let Event::Key(key) = event::read()? {
            if app_state.is_searching {
                match key.code {
                    KeyCode::Esc => app_state.is_searching = false,
                    KeyCode::Enter => app_state.execute_search(),
                    KeyCode::Backspace => { app_state.search_query.pop(); },
                    KeyCode::Char(c) => app_state.search_query.push(c),
                    _ => {}
                }
                continue;
            }

            if app_state.view_mode != gitloom::app::ViewMode::Graph {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => app_state.close_details(),
                    KeyCode::Char('j') | KeyCode::Down => {
                        if app_state.view_mode == gitloom::app::ViewMode::Refs {
                            app_state.scroll_refs_down();
                        } else {
                            app_state.scroll_details_down();
                        }
                    },
                    KeyCode::Char('k') | KeyCode::Up => {
                        if app_state.view_mode == gitloom::app::ViewMode::Refs {
                            app_state.scroll_refs_up();
                        } else {
                            app_state.scroll_details_up();
                        }
                    },
                    KeyCode::Right | KeyCode::Char('l') => app_state.scroll_details_right(),
                    KeyCode::Left | KeyCode::Char('h') => app_state.scroll_details_left(),
                    KeyCode::Enter => {
                        if app_state.view_mode == gitloom::app::ViewMode::Refs {
                            app_state.close_details();
                        } else {
                            app_state.load_details();
                        }
                    },
                    KeyCode::Char('f') => app_state.load_files(),
                    KeyCode::Char('d') => app_state.load_diff(),
                    KeyCode::Char('b') => app_state.load_refs(),
                    KeyCode::Char('/') => app_state.start_search(),
                    KeyCode::Char('n') => app_state.next_search_result(),
                    KeyCode::Char('N') => app_state.previous_search_result(),
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') => app_state.quit = true,
                    KeyCode::Char('j') | KeyCode::Down => app_state.next_commit(),
                    KeyCode::Char('k') | KeyCode::Up => app_state.previous_commit(),
                    KeyCode::Enter => app_state.load_details(),
                    KeyCode::Char('f') => app_state.load_files(),
                    KeyCode::Char('d') => app_state.load_diff(),
                    KeyCode::Char('b') => app_state.load_refs(),
                    KeyCode::Char('/') => app_state.start_search(),
                    KeyCode::Char('n') => app_state.next_search_result(),
                    KeyCode::Char('N') => app_state.previous_search_result(),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
