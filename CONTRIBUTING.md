# Contributing to GitLoom

First off, thank you for considering contributing to GitLoom! It's people like you that make this tool great.

## Development Setup

1. Make sure you have [Rust](https://rustup.rs/) installed.
2. Clone the repository:
   ```bash
   git clone https://github.com/yourusername/GitLoom.git
   cd GitLoom
   ```
3. Run the tests to ensure everything is working:
   ```bash
   cargo test
   ```

## Architecture Overview

GitLoom is a terminal-based Git history explorer built with `ratatui` and `libgit2`. 

- `src/main.rs`: Entry point and main event loop (using crossterm).
- `src/app/state.rs`: The core `AppState` that manages UI state, currently selected commit, active view modes, and search results.
- `src/git/repository.rs`: Wrapper around `git2::Repository` to fetch commits, generate tree diffs, and query branch/tag information.
- `src/graph/layout.rs`: The topological graph engine. This handles lane allocation, merge rendering, and DAG traversal.
- `src/ui/mod.rs`: Handles drawing the TUI widgets via Ratatui.

## Pull Request Process

1. Fork the repo and create your branch from `main`.
2. If you've added code that should be tested, add unit tests (especially in `src/graph/layout.rs` if modifying graph logic).
3. Ensure the test suite passes: `cargo test`.
4. Run `cargo fmt` to format your code.
5. Run `cargo clippy` and ensure no new warnings are introduced.
6. Commit using [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
   - `feat:` for new features
   - `fix:` for bug fixes
   - `docs:` for documentation changes
   - `refactor:` for code refactoring
7. Open your pull request!

## Code Style

- We enforce standard `rustfmt` styling.
- Avoid introducing `unwrap()` or `expect()` in UI rendering loops to prevent the TUI from abruptly crashing. Always handle `Result` gracefully where possible.

## Feature Roadmap

We are currently following a strict phase-by-phase implementation plan. Before picking up a large feature, please open an issue to discuss it or verify it aligns with the upcoming phases. [TODO](TODO)

Thank you for contributing!
