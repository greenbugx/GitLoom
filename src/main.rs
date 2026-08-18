pub mod app;
pub mod git;
pub mod graph;
pub mod ui;

use app::AppState;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
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
            match key.code {
                KeyCode::Char('q') => app_state.quit = true,
                KeyCode::Char('j') | KeyCode::Down => app_state.next_commit(),
                KeyCode::Char('k') | KeyCode::Up => app_state.previous_commit(),
                _ => {}
            }
        }
    }
    Ok(())
}
