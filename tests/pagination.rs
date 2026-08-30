//! Exercises HEAD-history pagination end to end: `AppState` driven against a
//! real on-disk repository deep enough to force more than one page.
//!
//! `PAGE_SIZE` commits at a time is slow to build purely as fixture setup
//! (thousands of real git objects written to disk), so this file builds one
//! `PAGE_SIZE + EXTRA`-deep repository per test rather than sharing a single
//! instance across tests: `TestRepo`'s `TempDir` is torn down at the end of
//! each test function, and a shared fixture would need it kept alive for the
//! whole file, which would leak temp directories on a panic anywhere in the
//! middle of the suite.

mod common;

use common::TestRepo;
use gitloom::app::loading::PAGE_SIZE;
use gitloom::app::{AppState, HistoryScope, RepoState};
use std::time::{Duration, Instant};

/// How far past one full page the fixtures in this file go, so a second page
/// is real but short. okayy :)
const EXTRA: usize = 250;

/// Spin `poll_load`/`poll_detail` until `condition` holds or `seconds` pass,
/// sleeping briefly between polls. Panics with `message` on timeout, the
/// same shape every wait loop in `tests/git_repository.rs` already uses.
/// Both are polled on every iteration, matching `main.rs`'s real event loop,
/// since scoping to all branches or a file's history answers through
/// `poll_detail` while HEAD pagination answers through `poll_load`.
fn wait_for(
    state: &mut AppState,
    seconds: u64,
    message: &str,
    mut condition: impl FnMut(&AppState) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !condition(state) {
        state.poll_load();
        state.poll_detail();
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A fresh `git init` has an unborn HEAD: no commit exists for the history
/// walk to even seed from. That must open as an empty, already-exhausted
/// history with the repository loaded — what the startup loader did before
/// pagination, flattening the walk failure into an empty commit list — not
/// as a full-screen load error.
#[test]
fn an_empty_repository_loads_as_an_empty_history() {
    let repo = TestRepo::init();
    let mut state = AppState::new(Some(repo.path().to_path_buf()), false);

    wait_for(
        &mut state,
        30,
        "empty repository load never finished",
        |s| s.loading.is_none(),
    );

    assert!(
        matches!(state.repo_state, RepoState::Loaded(..)),
        "an unborn HEAD should open the repository, not fail the load"
    );
    assert!(
        state.commits.is_empty(),
        "an empty repository has no commits to show"
    );
    assert_eq!(
        state.graph_rows.len(),
        state.commits.len(),
        "graph rows must stay one-per-commit even when that is zero"
    );
    assert!(state.minimap.is_empty());
    assert!(
        state.history_exhausted,
        "nothing can follow the empty first page"
    );
}

/// The first page alone should land within `PAGE_SIZE` commits, not the
/// repository's full depth: pagination should not have silently reverted to
/// "load everything up front" behavior.
#[test]
fn initial_load_stops_at_one_page() {
    let (repo, oids) = TestRepo::build_long_history_fixture(PAGE_SIZE + EXTRA);
    let mut state = AppState::new(Some(repo.path().to_path_buf()), false);

    wait_for(
        &mut state,
        30,
        "startup load never produced a first page",
        |s| !s.commits.is_empty(),
    );

    assert_eq!(
        state.commits.len(),
        PAGE_SIZE,
        "the first page should be exactly PAGE_SIZE commits, not the whole repository"
    );
    assert!(
        !state.history_exhausted,
        "a repository deeper than one page should not report itself exhausted yet"
    );
    // The newest PAGE_SIZE commits are the tip end of the walk: the fixture's
    // very last-created commit (deepest index) is HEAD, so it must be first.
    assert_eq!(state.commits[0].oid, *oids.last().unwrap());
}

/// Scrolling down to near the end of the first page should trigger a
/// background fetch of the next one, which should append rather than
/// replace, and the graph/minimap vectors should stay the same length as
/// `commits` throughout.
#[test]
fn scrolling_near_the_end_appends_the_next_page() {
    let (repo, oids) = TestRepo::build_long_history_fixture(PAGE_SIZE + EXTRA);
    let mut state = AppState::new(Some(repo.path().to_path_buf()), false);

    wait_for(&mut state, 30, "startup load never finished", |s| {
        !s.commits.is_empty()
    });
    assert_eq!(state.commits.len(), PAGE_SIZE);

    // Land on the last loaded commit: exactly the moment `go_last` should
    // notice the selection is at the edge of what's loaded and ask for more.
    state.go_last();
    assert!(
        state.loading_more,
        "reaching the end of the first page should start a background fetch"
    );

    wait_for(&mut state, 30, "second page never arrived", |s| {
        !s.loading_more
    });

    assert_eq!(
        state.commits.len(),
        PAGE_SIZE + EXTRA,
        "the second page should be appended onto the first, not replace it"
    );
    assert!(
        state.history_exhausted,
        "a page shorter than PAGE_SIZE should mark the walk exhausted"
    );
    assert_eq!(
        state.graph_rows.len(),
        state.commits.len(),
        "graph rows must stay one-per-commit after an append, per the \
         invariant documented in CONTRIBUTING.md"
    );
    assert_eq!(
        state.minimap.len(),
        state.commits.len(),
        "the minimap must extend in step with commits, not just the graph rows"
    );
    // The oldest commit in the whole fixture (the root, first-created) should
    // now be the very last entry: the walk reached all the way back.
    assert_eq!(state.commits.last().unwrap().oid, oids[0]);
}

/// The graph engine's lane state must carry across the page boundary rather
/// than reset: a purely linear history should render as a single unbroken
/// lane the whole way through, not restart its topology at commit 1000.
#[test]
fn graph_lanes_stay_continuous_across_a_page_boundary() {
    let (repo, _oids) = TestRepo::build_long_history_fixture(PAGE_SIZE + EXTRA);
    let mut state = AppState::new(Some(repo.path().to_path_buf()), false);

    wait_for(&mut state, 30, "startup load never finished", |s| {
        !s.commits.is_empty()
    });
    state.go_last();
    wait_for(&mut state, 30, "second page never arrived", |s| {
        !s.loading_more
    });

    // A linear history has exactly one lane throughout; every row plain "●"
    // is `GraphEngine`'s rendering for a single-parent, single-child commit
    // with nothing else sharing the lane (see `tests/graph_layout.rs`'s
    // `test_linear_history` for the same assertion on a short, in-memory
    // history).
    for (i, row) in state.graph_rows.iter().enumerate() {
        assert_eq!(
            row.render_plain(),
            "●",
            "row {i} should still render as a single unbroken lane across the page boundary"
        );
    }
}

/// A search for a commit that only exists past the first page should not
/// report "not found": it should keep loading pages until it turns up or the
/// walk is genuinely exhausted, matching the issue this feature answers
/// ("searching for an older commit can incorrectly report that it was not
/// found").
#[test]
fn search_continues_past_the_first_page() {
    let (repo, oids) = TestRepo::build_long_history_fixture(PAGE_SIZE + EXTRA);
    let mut state = AppState::new(Some(repo.path().to_path_buf()), false);

    wait_for(&mut state, 30, "startup load never finished", |s| {
        !s.commits.is_empty()
    });

    // The root commit (index 0) is the oldest, and with EXTRA = 250 it sits
    // well past the first PAGE_SIZE commits — exactly the case that used to
    // be unreachable.
    let target_oid = &oids[0];
    assert!(
        !state.commits.iter().any(|c| &c.oid == target_oid),
        "sanity check on the fixture: the target must not already be on the first page"
    );

    state.search_query = target_oid.clone();
    state.execute_search();

    wait_for(
        &mut state,
        30,
        "search never resolved past the first page",
        |s| !s.search_results.is_empty(),
    );

    assert_eq!(
        state.commits.len(),
        PAGE_SIZE + EXTRA,
        "search should have paged in the rest of history"
    );
    let found_index = *state.search_results.first().unwrap();
    assert_eq!(&state.commits[found_index].oid, target_oid);
}

/// Searching for something that genuinely does not exist anywhere in history
/// should still terminate in a normal "not found", not hang forever waiting
/// on pages that will never come once the walk is exhausted.
#[test]
fn search_for_a_missing_commit_terminates_once_exhausted() {
    let (repo, _oids) = TestRepo::build_long_history_fixture(PAGE_SIZE + EXTRA);
    let mut state = AppState::new(Some(repo.path().to_path_buf()), false);

    wait_for(&mut state, 30, "startup load never finished", |s| {
        !s.commits.is_empty()
    });

    state.search_query = "this summary never appears in the fixture".to_string();
    state.execute_search();

    wait_for(
        &mut state,
        30,
        "search never terminated on a full miss",
        |s| s.history_exhausted && !s.loading_more,
    );

    assert!(
        state.search_results.is_empty(),
        "a query absent from the whole history should end with no results"
    );
    assert_eq!(
        state.commits.len(),
        PAGE_SIZE + EXTRA,
        "an exhaustive search should have paged in the entire history along the way"
    );
}

/// Scoping to all branches mid-pagination must not let a page meant for HEAD
/// land on the scoped list: `close_history` should restore the full,
/// correctly-appended HEAD history, not a HEAD list frozen at whatever size
/// it was when the scope was opened, nor a scoped list corrupted by a HEAD
/// page landing on top of it.
#[test]
fn a_page_in_flight_when_scoping_away_lands_on_the_parked_snapshot() {
    let (repo, oids) = TestRepo::build_long_history_fixture(PAGE_SIZE + EXTRA);
    let mut state = AppState::new(Some(repo.path().to_path_buf()), false);

    wait_for(&mut state, 30, "startup load never finished", |s| {
        !s.commits.is_empty()
    });

    // Trigger the second page. It is not awaited here on purpose — the rest
    // of this test deliberately never calls `poll_load` until well after
    // scoping away, so the `PageReady` this produces is still guaranteed to
    // be sitting unread in the channel (not yet applied to `AppState`) by
    // the time `self.history` changes below. That ordering is the entire
    // point of the test: it is what actually exercises the "page lands
    // after scoping away" branch in `apply`'s `PageReady` arm, rather than
    // leaving it to chance which of two background threads happens to
    // finish first.
    state.go_last();
    assert!(state.loading_more);

    // Deliberately `poll_detail` only, never `poll_load`, so the scope
    // change is applied without any chance of the still-pending HEAD page
    // being drained first. `toggle_all_branches_history` from `Head` goes
    // straight to `Request::AllBranchesHistory` (see
    // `tests/git_repository.rs`'s `an_all_session_first_a_press_fetches_head_history`
    // for the mirror image of this under `--all`), answered through
    // `poll_detail`, never `poll_load`.
    state.toggle_all_branches_history();
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.history != HistoryScope::AllBranches {
        state.poll_detail();
        assert!(
            Instant::now() < deadline,
            "all-branches request never resolved"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // Only now does `poll_load` run for the first time since `go_last`,
    // with `self.history` already `AllBranches`.
    wait_for(
        &mut state,
        30,
        "HEAD page never finished loading in the background",
        |s| !s.loading_more,
    );
    assert_eq!(
        state.history,
        HistoryScope::AllBranches,
        "receiving the background HEAD page must not itself change the scope"
    );
    assert!(
        state.commits.iter().all(|c| oids.contains(&c.oid)),
        "the all-branches list on screen should be undisturbed by the HEAD page"
    );

    let closed = state.close_history();
    assert!(closed);
    assert_eq!(state.history, HistoryScope::Head);
    assert_eq!(
        state.commits.len(),
        PAGE_SIZE + EXTRA,
        "closing back to HEAD should show the fully paginated history, including \
         the page that arrived while scoped to all branches"
    );
    assert_eq!(state.commits.last().unwrap().oid, oids[0]);
    assert_eq!(
        state.graph_rows.len(),
        state.commits.len(),
        "graph rows must still be aligned with commits after being spliced back in"
    );
}
