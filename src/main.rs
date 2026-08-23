use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gitloom::app::{AppState, ViewMode};
use gitloom::ui;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    error::Error,
    io,
    time::{Duration, Instant},
};

const TICK: Duration = Duration::from_millis(80);

fn main() -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).map(std::path::PathBuf::from);

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
    let mut last_tick = Instant::now();

    while !app_state.quit {
        app_state.poll_load();

        terminal
            .draw(|f| ui::render(f, app_state))
            .map_err(|e| e.into())?;

        if app_state.is_loading() {
            if event::poll(TICK.saturating_sub(last_tick.elapsed()))?
                && let Event::Key(key) = event::read()?
            {
                handle_key(app_state, key);
            }

            if last_tick.elapsed() >= TICK {
                app_state.tick();
                last_tick = Instant::now();
            }
        } else if let Event::Key(key) = event::read()? {
            handle_key(app_state, key);
        }
    }
    Ok(())
}

fn handle_key(app_state: &mut AppState, key: KeyEvent) {
    // Windows reports a press AND a release per keystroke; without this every
    // key would act twice. Unix only sends `Press`, so this costs nothing there.
    if key.kind == KeyEventKind::Release {
        return;
    }
    let code = key.code;

    if app_state.commits.is_empty() && app_state.is_loading() {
        if matches!(code, KeyCode::Char('q') | KeyCode::Esc) {
            app_state.quit = true;
        }
        return;
    }

    if app_state.is_searching {
        match code {
            KeyCode::Esc => app_state.is_searching = false,
            KeyCode::Enter => app_state.execute_search(),
            KeyCode::Backspace => {
                app_state.search_query.pop();
            }
            KeyCode::Char(c) => app_state.search_query.push(c),
            _ => {}
        }
        return;
    }

    // A single table of (view_mode, key) instead of two match arms that each
    // hand-list f/d/b/n/N/etc. and then re-test the view mode inside
    // themselves. Every binding lives in exactly one place now, so the Graph
    // and detail-pane arms can't quietly drift apart from each other.
    // `view_mode` is copied out first (it's a plain enum, no data) so the
    // match arms are free to take `&mut app_state`.
    let view_mode = app_state.view_mode;
    match (view_mode, code) {
        (ViewMode::Graph, KeyCode::Char('q')) => app_state.quit = true,
        (_, KeyCode::Esc | KeyCode::Char('q')) => app_state.close_details(),

        (ViewMode::Graph, KeyCode::Char('j') | KeyCode::Down) => app_state.next_commit(),
        (ViewMode::Graph, KeyCode::Char('k') | KeyCode::Up) => app_state.previous_commit(),
        (ViewMode::Refs, KeyCode::Char('j') | KeyCode::Down) => app_state.scroll_refs_down(),
        (ViewMode::Refs, KeyCode::Char('k') | KeyCode::Up) => app_state.scroll_refs_up(),
        (_, KeyCode::Char('j') | KeyCode::Down) => app_state.scroll_details_down(),
        (_, KeyCode::Char('k') | KeyCode::Up) => app_state.scroll_details_up(),

        (_, KeyCode::Right | KeyCode::Char('l')) => app_state.scroll_details_right(),
        (_, KeyCode::Left | KeyCode::Char('h')) => app_state.scroll_details_left(),

        (ViewMode::Refs, KeyCode::Enter) => app_state.close_details(),
        (_, KeyCode::Enter) => app_state.load_details(),

        (_, KeyCode::Char('f')) => app_state.load_files(),
        (_, KeyCode::Char('d')) => app_state.load_diff(),
        (_, KeyCode::Char('b')) => app_state.load_refs(),
        (_, KeyCode::Char('/')) => app_state.start_search(),
        (_, KeyCode::Char('n')) => app_state.next_search_result(),
        (_, KeyCode::Char('N')) => app_state.previous_search_result(),
        _ => {}
    }
}
