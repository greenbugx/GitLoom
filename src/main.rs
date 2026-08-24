use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use gitloom::app::{AppState, ViewMode};
use gitloom::cli::{self, Cli};
use gitloom::ui;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    error::Error,
    io,
    process::ExitCode,
    time::{Duration, Instant},
};

const TICK: Duration = Duration::from_millis(80);

fn main() -> ExitCode {
    // Arguments are settled before the terminal is touched, so `--help` and a
    // bad flag both print to a normal terminal instead of into the alternate
    // screen, and neither reaches `Repository::discover`.
    let path = match cli::parse(std::env::args()) {
        Ok(Cli::Run(path)) => path,
        Ok(Cli::Help) => {
            println!("{}", cli::HELP);
            return ExitCode::SUCCESS;
        }
        Ok(Cli::Version) => {
            println!("{}", cli::version());
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("gitloom: {message}");
            eprintln!("Try `gitloom --help` for usage.");
            return ExitCode::FAILURE;
        }
    };

    match run(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gitloom: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: Option<std::path::PathBuf>) -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app_state = AppState::new(path);

    let res = run_app(&mut terminal, &mut app_state);

    // Restore the terminal before reporting anything, or an error message
    // would be printed into the alternate screen and vanish with it.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
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
        app_state.poll_detail();

        terminal
            .draw(|f| ui::render(f, app_state))
            .map_err(|e| e.into())?;

        // `is_busy`, not `is_loading`: a detail or diff fetch also changes the
        // screen without a key press, so blocking on input would leave the
        // result sitting in the channel until the user happened to press
        // something.
        if app_state.is_busy() {
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

    // Ahead of the help overlay so a `?` typed into a query is a `?`.
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

    // The overlay is modal: it floats over whatever pane was open, so it takes
    // the keys that dismiss it and swallows the rest rather than letting a
    // stray `d` change the pane underneath.
    if app_state.show_help {
        if matches!(
            code,
            KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter
        ) {
            app_state.show_help = false;
        }
        return;
    }
    if code == KeyCode::Char('?') {
        app_state.show_help = true;
        return;
    }

    if app_state.commits.is_empty() && app_state.is_loading() {
        if matches!(code, KeyCode::Char('q') | KeyCode::Esc) {
            app_state.quit = true;
        }
        return;
    }

    // A single table of (view_mode, key) instead of two match arms that each
    // hand-list f/d/b/n/N/etc. and then re-test the view mode inside
    // themselves. Every binding lives in exactly one place now, so the Graph
    // and detail-pane arms can't quietly drift apart from each other.
    // `view_mode` is copied out first (it's a plain enum, no data) so the
    // match arms are free to take `&mut app_state`.
    //
    // Order matters: the pane-specific arms have to precede the `(_, key)`
    // catch-alls, which is also why `l` appears twice below.
    let view_mode = app_state.view_mode;
    match (view_mode, code) {
        (ViewMode::Graph, KeyCode::Char('q')) => app_state.quit = true,
        // In the graph pane Esc backs out of a file's history. With full
        // history already showing there is nothing to back out of, so it does
        // what Esc has always done here and clears any lingering message.
        (ViewMode::Graph, KeyCode::Esc) => {
            if !app_state.close_file_history() {
                app_state.close_details();
            }
        }
        (_, KeyCode::Esc | KeyCode::Char('q')) => app_state.close_details(),

        (ViewMode::Graph, KeyCode::Char('j') | KeyCode::Down) => app_state.next_commit(),
        (ViewMode::Graph, KeyCode::Char('k') | KeyCode::Up) => app_state.previous_commit(),
        (ViewMode::Refs, KeyCode::Char('j') | KeyCode::Down) => app_state.scroll_refs_down(),
        (ViewMode::Refs, KeyCode::Char('k') | KeyCode::Up) => app_state.scroll_refs_up(),
        (ViewMode::Files, KeyCode::Char('j') | KeyCode::Down) => app_state.next_file(),
        (ViewMode::Files, KeyCode::Char('k') | KeyCode::Up) => app_state.previous_file(),
        (_, KeyCode::Char('j') | KeyCode::Down) => app_state.scroll_details_down(),
        (_, KeyCode::Char('k') | KeyCode::Up) => app_state.scroll_details_up(),

        // Walk commits with a pane open, so a diff can be followed down the
        // history without closing and reopening it.
        (_, KeyCode::Char('J')) => app_state.step_commit_and_follow(1),
        (_, KeyCode::Char('K')) => app_state.step_commit_and_follow(-1),

        (_, KeyCode::Char('g') | KeyCode::Home) => app_state.go_first(),
        (_, KeyCode::Char('G') | KeyCode::End) => app_state.go_last(),
        (_, KeyCode::PageDown) => app_state.page(1),
        (_, KeyCode::PageUp) => app_state.page(-1),

        // `l` scopes the graph to the selected path. The files pane is a list
        // with nothing to scroll sideways, so the key (and Right, which would
        // otherwise move a scroll offset nothing reads) is free to descend.
        (ViewMode::Files, KeyCode::Char('l') | KeyCode::Right) => app_state.open_file_history(),
        (_, KeyCode::Right | KeyCode::Char('l')) => app_state.scroll_details_right(),
        (_, KeyCode::Left | KeyCode::Char('h')) => app_state.scroll_details_left(),

        (ViewMode::Refs, KeyCode::Enter) => app_state.close_details(),
        (_, KeyCode::Enter) => app_state.load_details(),

        (_, KeyCode::Char('f')) => app_state.load_files(),
        (_, KeyCode::Char('d')) => app_state.load_diff(),
        (_, KeyCode::Char('b')) => app_state.load_refs(),
        (_, KeyCode::Char('y')) => app_state.yank_selected_oid(),
        (_, KeyCode::Char('/')) => app_state.start_search(),
        (_, KeyCode::Char('n')) => app_state.next_search_result(),
        (_, KeyCode::Char('N')) => app_state.previous_search_result(),
        _ => {}
    }
}
