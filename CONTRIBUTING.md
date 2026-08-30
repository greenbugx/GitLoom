# Contributing to GitLoom

First off, thank you for considering contributing to GitLoom! It's people like you that make this tool great.

## Development Setup

1. Make sure you have [Rust](https://rustup.rs/) installed. GitLoom is on edition
   2024 and the event loop uses let-chains, so it needs **Rust 1.88 or newer**;
   the floor is declared as `rust-version` in `Cargo.toml` and enforced by CI, so
   cargo will tell you plainly if your toolchain is too old. `rustup update
   stable` fixes it.
2. Clone the repository:
   ```bash
   git clone https://github.com/greenbugx/GitLoom.git
   cd GitLoom
   ```
3. Run the tests to ensure everything is working:
   ```bash
   cargo test
   ```

## Architecture Overview

GitLoom is a terminal-based Git history explorer built with `ratatui` and
`libgit2`. Almost all of it lives in a library crate (`src/lib.rs`) with a thin
binary on top, so the integration tests in `tests/` drive the same code the TUI
does.

**Entry point**

- `src/main.rs`: The main event loop, and the single `(view_mode, key)` table
  that every binding lives in.
- `src/cli.rs`: Argument parsing, hand-rolled and dependency-free. Runs before
  the terminal enters raw mode so `--help` prints to a normal screen.
- `src/terminal.rs`: Raw mode and the alternate screen, entered through an RAII
  guard so they are undone on every exit path, panics included.

**Application state**

- `src/app/state.rs`: The core `AppState` that manages UI state, currently
  selected commit, active view modes, search results, and the graph's history
  scope via `HistoryScope` (HEAD, all local branches, or a single file's
  history). Scoped views park the previous commit list in a `HistorySnapshot`
  so closing restores the original list and selection without re-walking.
- `src/app/detail.rs`: The worker thread that fetches commit details, diffs, and
  changed-file lists off the UI thread.
- `src/app/loading.rs`: The worker that walks history when a repository is
  opened. HEAD history is paginated: it loads one `PAGE_SIZE`-commit page up
  front, then stays alive and answers `LoadRequest::NextPage` by resuming the
  same `git2::Revwalk`, so the app is never stuck with a fixed ceiling on how
  much history it can reach. An `--all` startup load is still one-shot.
- `src/app/details_text.rs`: Formats a commit into the details pane's lines.

**Git layer**

- `src/git/repository.rs`: Wrapper around `git2::Repository` to fetch commits,
  generate tree diffs, query branch/tag information, and walk a single path's
  history.
- `src/git/commit.rs`: Owned commit snapshots, so commit data can cross a thread
  boundary (`git2` types are not `Send`).

**Rendering**

- `src/graph/layout.rs`: The topological graph engine. This handles lane
  allocation, merge rendering, and DAG traversal.
- `src/ui/mod.rs`: Handles drawing the TUI widgets via Ratatui.
- `src/ui/help.rs`: The `?` overlay, and the keymap table itself.
- `src/ui/loading.rs`: The loading animation.
- `src/clipboard.rs`: OSC 52 clipboard writes.

**Test infrastructure**

- `tests/common/mod.rs`: Builds small, real repositories on disk with `git2`
  (the `TestRepo` builder) so integration tests exercise the git layer,
  search, minimap, and graph parent-handling against actual commit/tree/ref
  objects instead of only hand-built `CommitInfo`s. Also builds the deep
  linear history (`build_long_history_fixture`) used by the ignored revwalk
  benchmark and by `tests/pagination.rs`.
- `tests/graph_layout.rs`: Pure topology tests for `GraphEngine::build` —
  linear histories, single and multiple branches, merges, and complicated
  merge topologies.
- `tests/git_repository.rs`: Integration tests that open a real repository
  and exercise the git layer end-to-end: commit walks, topological sort
  guarantees, ref map badges, file history (including mode-only changes and
  unmerged branches), search, minimap alignment, and the `--all` startup
  flag.
- `tests/pagination.rs`: Drives `AppState` against a repository deep enough
  to force more than one page — the page boundary, graph/minimap staying
  aligned with `commits` after an append, search continuing past the first
  page instead of reporting "not found" early, and the race where a page
  lands after the graph has scoped away from HEAD.

## Invariants worth knowing

A handful of rules are load-bearing, enforced by tests, and easy to trip over:

- **The keymap is data.** `ui::help::SECTIONS` is the only place the bindings are
  published to users, and a test asserts that every key `main.rs` handles appears
  in the *key column* of that table — putting it in a description doesn't count.
  Add a binding in both places.
- **One `GraphRow` per commit.** The layout inserts no spacer rows, so
  `AppState.list_state` indexes straight into `commits[i]`. Diagonal and corner
  connectors are compressed into the commit's own row; the module comment in
  `src/graph/layout.rs` explains the tradeoff.
- **Filtered histories use `GraphEngine::build_linear`, not `build`.** `build`
  holds a lane open until the parent it is waiting for arrives, so a list with
  gaps in it — a single file's history, for instance — widens into a staircase,
  one lane per row.
- **The detail worker's three guarantees are separate.** Request coalescing bounds
  the cost of a keypress burst, sequence numbers stop a slow fetch from
  repainting a pane the user has already left, and `cancel()` stops a closed pane
  being repopulated. Each has its own test in `src/app/detail.rs`; please don't
  collapse them into one mechanism.
- **`LoadWorker` page requests accumulate; they are never coalesced.** This
  is the opposite of `DetailWorker`'s rule above, and deliberately so:
  coalescing keeps only the newest of a backlog, which is correct for "redraw
  the pane for the commit I'm on now" but wrong for pagination — two
  `NextPage` requests sent before the first is answered both need a page
  back, or the second page silently never loads and history looks shorter
  than it is.
- **A HEAD-history page can land after the graph has scoped away from it.**
  If the user scopes to a file's history or all branches while a `NextPage`
  request is still in flight, the page that eventually arrives must go into
  the parked `HistorySnapshot`, not `AppState.commits` — which by then holds
  the scoped list, not HEAD's. `apply`'s `LoadMessage::PageReady` arm checks
  `history != HistoryScope::Head` for exactly this reason before deciding
  where to append.
- **The terminal is restored by a `Drop` guard, not by code at the end of
  `run`.** `run` holds it as `let _guard = TerminalGuard::enter()?`; the `_guard`
  name is load-bearing, since `let _ = ` drops the value immediately and would
  leave raw mode before the first frame. `enter` installs the panic hook itself
  and only once per process (a `PANIC_HOOK_INSTALLED` flag no-ops a second
  install), so the thread that owns the terminal is by construction the one
  thread the hook restores for. It restores for that thread only — a worker
  thread dying leaves the TUI running, and pulling the screen out from under it
  would make a background failure look like a rendering bug.
- **Line endings are LF.** `.gitattributes` normalizes on checkin. If you develop
  across Windows and a Linux container, review with
  `git diff --ignore-cr-at-eol` so real changes aren't buried in `^M`.
- **Scoped history parks the previous view.** `HistoryScope::File` and
  `HistoryScope::AllBranches` replace the commit list rather than filtering it,
  because the graph rows, minimap, and selection are all index-aligned with
  `commits` and would need recomputation otherwise. A `HistorySnapshot` holds
  the parked view; only the first scope open saves a snapshot, so switching
  from one scope to another (e.g. file history to all-branches) does not
  overwrite the snapshot. `close_history` restores the parked view exactly,
  selection included. `toggle_all_branches_history` uses this mechanism so
  repeated presses swap between the two views without re-walking.

## Pull Request Process

1. Fork the repo and create your branch from `main`.
2. If you've added code that should be tested, add unit tests (especially in
   `src/graph/layout.rs` if modifying graph logic).
3. Ensure the test suite passes: `cargo test`.
4. Run `cargo fmt` to format your code. CI runs `cargo fmt --check`.
5. Run `cargo clippy --all-targets -- -D warnings`. CI treats every warning as an
   error, including unused imports.
6. Commit using [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
   - `feat:` for new features
   - `fix:` for bug fixes
   - `docs:` for documentation changes
   - `refactor:` for code refactoring
   - `test:` for tests
   - `ci:` for CI and workflow changes
   - `chore:` for everything else
7. Open your pull request! A
   [pull request template](.github/pull_request_template.md) is provided —
   fill in what applies. Use the
   [bug report](.github/ISSUE_TEMPLATE/bug_report.md) or
   [feature request](.github/ISSUE_TEMPLATE/feature_request.md) templates
   when opening issues.

CI runs three jobs. Build and test go across `ubuntu-latest` and
`windows-latest`, because the event loop is platform-sensitive: Windows reports a
key press *and* a release per keystroke, and `main.rs` filters the release out.
Format and clippy run once on Linux, since neither depends on the platform. A
third job checks the crate still compiles on the `rust-version` floor declared in
`Cargo.toml`, so that number stays honest.

The one `#[ignore]`d test is a revwalk benchmark that takes a few minutes and
doesn't run in CI; run it explicitly with `cargo test -- --ignored --nocapture`
if you touch the loading path.

## Code Style

- We enforce standard `rustfmt` styling.
- Avoid introducing `unwrap()` or `expect()` in UI rendering loops to prevent the TUI from abruptly crashing. Always handle `Result` gracefully where possible.

## Feature Roadmap

We are currently following a strict phase-by-phase implementation plan. Before
picking up a large feature, please open an issue to discuss it, so we can check
it lines up with what's next.

Thank you for contributing!
