//! Exercises `GitRepository`, the minimap, and search against a real
//! on-disk repository (built by `tests/common`) instead of hand-built
//! `CommitInfo`s. `tests/graph_layout.rs` already covers the graph engine's
//! pure topology math in isolation; these tests cover the seam where that
//! data actually comes from libgit2 and where `AppState` puts it together.

mod common;

use common::TestRepo;
use gitloom::app::AppState;
use gitloom::app::ref_pane_rows;
use gitloom::git::repository::{Branch, GitRepository, Ref, RefName, WalkStart};
use gitloom::graph::layout::GraphEngine;
use std::collections::HashSet;

/// `commits_with_progress` should walk every commit reachable from HEAD,
/// each with the right parent count, and the merge commit should report
/// both of its parents.
#[test]
fn walks_every_commit_with_correct_parents() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let commits = repo.commits(100).expect("walk commit history");
    assert_eq!(commits.len(), 5, "root, base, main_tip, feature, merge");

    let by_oid: std::collections::HashMap<_, _> =
        commits.iter().map(|c| (c.oid.clone(), c)).collect();

    let root = by_oid.get(&fixture.root).expect("root commit present");
    assert!(root.parents.is_empty(), "root commit has no parents");

    let merge = by_oid.get(&fixture.merge).expect("merge commit present");
    assert_eq!(
        merge.parents.len(),
        2,
        "merge commit should keep both parents, not just parent(0)"
    );
    assert!(merge.parents.contains(&fixture.main_tip));
    assert!(merge.parents.contains(&fixture.feature));

    let feature = by_oid
        .get(&fixture.feature)
        .expect("feature commit present");
    assert_eq!(feature.parents, vec![fixture.base.clone()]);
}

/// The revwalk is `TOPOLOGICAL | TIME`: every commit must come before its
/// parents in the returned order, regardless of how ties between sibling
/// branches are broken.
#[test]
fn topological_sort_always_puts_children_before_parents() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");
    let commits = repo.commits(100).expect("walk commit history");

    let position: std::collections::HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.oid.as_str(), i))
        .collect();

    for commit in &commits {
        for parent_oid in &commit.parents {
            let child_pos = position[commit.oid.as_str()];
            let parent_pos = position[parent_oid.as_str()];
            assert!(
                child_pos < parent_pos,
                "{} should be walked before its parent {}",
                commit.oid,
                parent_oid
            );
        }
    }

    // The two guarantees `TOPOLOGICAL | TIME` actually gives us, independent
    // of libgit2's tie-breaking between the two branches in between.
    assert_eq!(commits[0].oid, fixture.merge, "merge is the tip of main");
    assert_eq!(
        commits[commits.len() - 1].oid,
        fixture.root,
        "root has no parents, so it must sort last"
    );
}

/// `GraphEngine::build` should place the merge commit's node in a way that
/// reflects two active lanes at the point of the merge (one per parent),
/// exercising real merge topology rather than the hand-built cases in
/// `tests/graph_layout.rs`.
#[test]
fn graph_engine_sees_a_real_merge_as_two_lanes() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");
    let commits = repo.commits(100).expect("walk commit history");
    let rows = GraphEngine::build(&commits);

    assert_eq!(rows.len(), commits.len());

    let merge_row = rows
        .iter()
        .find(|r| r.commit_oid == fixture.merge)
        .expect("graph row for the merge commit");
    let active_lanes = merge_row
        .segments
        .iter()
        .filter(|s| !matches!(s.glyph, gitloom::graph::layout::GlyphType::Empty))
        .count();
    assert!(
        active_lanes >= 2,
        "a merge commit should occupy at least two lanes at the point of the merge, got {active_lanes}"
    );
}

/// `ref_map` should attach the branch and tag badges to the exact commits
/// they point at, and nowhere else.
#[test]
fn ref_map_attaches_badges_to_the_right_commits() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");
    let ref_map = repo.ref_map().expect("build ref map");

    let root_badges = ref_map
        .get(&fixture.root)
        .expect("root has a badge (the tag)");
    assert!(
        root_badges
            .iter()
            .any(|b| matches!(b, Ref::Tag(name) if name.as_str() == "v0.1.0")),
        "root should carry the v0.1.0 tag badge"
    );

    let merge_badges = ref_map
        .get(&fixture.merge)
        .expect("merge is main's tip and should carry the main badge");
    assert!(
        merge_badges
            .iter()
            .any(|b| matches!(b, Ref::Branch(Branch::Local(name)) if name.as_str() == "main")),
        "merge should carry the local `main` branch badge"
    );

    let feature_badges = ref_map
        .get(&fixture.feature)
        .expect("feature tip should carry the feature badge");
    assert!(
        feature_badges
            .iter()
            .any(|b| matches!(b, Ref::Branch(Branch::Local(name)) if name.as_str() == "feature"))
    );

    // base is an ancestor of both tips but isn't itself pointed at by any
    // ref, so it should carry no badges at all.
    assert!(
        !ref_map.contains_key(&fixture.base),
        "an ancestor commit with no ref pointing at it should have no badges"
    );
}

#[test]
fn refs_lists_branches_and_tags_and_rows_flattens_them() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");
    let refs = repo.refs().expect("list refs");

    assert!(refs.branches.contains(&Branch::Local(RefName::new("main"))));
    assert!(
        refs.branches
            .contains(&Branch::Local(RefName::new("feature")))
    );
    assert!(refs.tags.contains(&RefName::new("v0.1.0")));

    let rows = ref_pane_rows(&refs);
    let header_count = rows.iter().filter(|r| r.is_header()).count();
    assert_eq!(header_count, 3, "Local Branches / Remote Branches / Tags");

    let entry_count = rows.iter().filter(|r| !r.is_header()).count();
    assert_eq!(
        entry_count,
        refs.branches.len() + refs.tags.len(),
        "every branch and tag should appear as exactly one row"
    );
}

/// The minimap has one char per commit and is index-aligned with the
/// commit list it was built from, end to end through `commit_deltas`.
#[test]
fn minimap_stays_aligned_with_the_commit_list() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");
    let commits = repo.commits(100).expect("walk commit history");

    let oids: Vec<String> = commits.iter().map(|c| c.oid.clone()).collect();
    let deltas = repo.commit_deltas(&oids);
    assert_eq!(
        deltas.len(),
        commits.len(),
        "one delta per commit, same order"
    );

    let minimap = gitloom::app::loading::minimap_chars(&deltas);
    assert_eq!(
        minimap.len(),
        commits.len(),
        "one minimap glyph per commit, same order"
    );

    // The root commit only adds a README; the merge commit's diff is empty
    // against its first parent's tree only if nothing changed there, so
    // instead just assert every position has *some* sparkline character
    // rather than asserting exact bucket values, which would over-specify
    // libgit2's diff stats.
    const SPARKLINE_CHARS: &str = "▁▂▃▄▅▆▇█";
    for ch in &minimap {
        assert!(
            SPARKLINE_CHARS.contains(*ch),
            "{ch} should be one of the defined sparkline glyphs"
        );
    }
}

/// `AppState::execute_search` should find commits by summary, author, and
/// oid substring against real loaded data, and land the selection on the
/// first match.
#[test]
fn search_finds_commits_by_summary_author_and_oid() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    // Constructed against the fixture's own path (not `AppState::default()`,
    // which discovers a repo from the test process's current directory
    // usually the GitLoom checkout itself, not what this test wants) so the
    // background loader and repo_state reflect the fixture, not the
    // repository this test suite happens to live in.
    let mut state = AppState::new(Some(fixture.repo_path.clone()), false);
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");
    state.commits = repo.commits(100).expect("walk commit history");

    state.search_query = "handle empty input".to_string();
    state.execute_search();
    assert_eq!(state.search_results.len(), 1);
    let found = &state.commits[state.search_results[0]];
    assert_eq!(found.oid, fixture.main_tip);
    assert_eq!(state.list_state.selected(), Some(state.search_results[0]));

    state.search_query = "test author".to_string();
    state.execute_search();
    assert_eq!(
        state.search_results.len(),
        5,
        "every fixture commit shares the same author"
    );

    state.search_query = fixture.merge[..7].to_string();
    state.execute_search();
    assert_eq!(state.search_results.len(), 1);
    assert_eq!(state.commits[state.search_results[0]].oid, fixture.merge);

    state.search_query = "no such commit exists anywhere".to_string();
    state.execute_search();
    assert!(state.search_results.is_empty());
}

/// `changed_files` and `commit_diff` should reflect the file actually
/// touched by a commit against its first parent.
#[test]
fn changed_files_and_diff_reflect_the_real_commit_content() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let files = repo
        .changed_files(&fixture.feature)
        .expect("list files changed on the feature commit");
    assert_eq!(files, vec!["src/feature.rs".to_string()]);

    let diff = repo
        .commit_diff(&fixture.feature)
        .expect("diff the feature commit");
    assert!(
        diff.iter().any(|line| line.contains("pub fn greet")),
        "diff should include the added function"
    );
    assert!(
        diff.iter().any(|line| line.starts_with('+')),
        "diff should have at least one added line"
    );
}

/// This test doesn't assert a pass/fail either way — it's a measurement,
/// printed so `cargo test -- --nocapture` surfaces it, run with
/// `#[ignore]` since it's a benchmark, not a correctness check. A long,
/// mostly-linear history is built and timed with `max_count` set both far
/// below and at the total history size, repeated a few times and averaged
/// (with an untimed warm-up pass first) so OS filesystem cache effects
/// don't swamp the signal we actually care about.
///
/// Reading the result: if `take(small)` averages close to `take(all)`, the
/// revwalk is paying an up-front O(history) cost regardless of how few
/// commits are requested, meaning `COMMIT_LIMIT` bounds memory but not this
/// part of the load time, and that cost not diffing or graph layout is
/// plausibly the next thing worth optimizing on a large repo. If
/// `take(small)` is a small fraction of `take(all)`, the walk is lazy
/// enough that `COMMIT_LIMIT` is already doing the job here.
#[test]
#[ignore = "benchmark, not a correctness check; run explicitly with `cargo test -- --ignored --nocapture`"]
fn measure_whether_revwalk_pays_full_history_cost_up_front() {
    use std::time::{Duration, Instant};

    const DEPTH: usize = 5_000;
    const REPETITIONS: u32 = 5;

    let repo_dir = build_linear_history(DEPTH);
    let repo_path = repo_dir.path().to_path_buf();

    let git_repo = GitRepository::open(&repo_path).expect("open linear-history repo");
    let small_take = 20;

    // Untimed warm-up: pays the first-touch filesystem cache cost once, up
    // front, so it doesn't get attributed to whichever case happens to run
    // first below.
    git_repo
        .commits(DEPTH + 1)
        .expect("warm-up walk of the entire history");

    let mut small_total = Duration::ZERO;
    let mut full_total = Duration::ZERO;
    for _ in 0..REPETITIONS {
        let start = Instant::now();
        let small = git_repo
            .commits(small_take)
            .expect("walk a small prefix of a long history");
        small_total += start.elapsed();
        assert_eq!(small.len(), small_take);

        let start = Instant::now();
        let full = git_repo
            .commits(DEPTH + 1)
            .expect("walk the entire history");
        full_total += start.elapsed();
        assert_eq!(full.len(), DEPTH, "root plus DEPTH - 1 descendants");
    }

    let small_avg = small_total / REPETITIONS;
    let full_avg = full_total / REPETITIONS;
    eprintln!(
        "revwalk timing over {DEPTH} linear commits, averaged over {REPETITIONS} reps:\n  \
         take({small_take})  = {small_avg:?}\n  \
         take(all={DEPTH}) = {full_avg:?}\n  \
         take(small) / take(all) = {:.4}  \
         (near 1.0 => full cost paid up front; near 0.0 => lazy)",
        small_avg.as_secs_f64() / full_avg.as_secs_f64().max(1e-9)
    );
}

/// Builds `depth` linear commits in total (the first is the root, with no
/// parent; the rest have exactly one parent each) on `main`, each touching
/// the same file so the diffs stay cheap and the timing reflects
/// revwalk/commit-object cost rather than diff cost. Returns the
/// `TempDir` only; the `git2::Repository` handle is dropped internally so
/// the caller reopens fresh from disk via `GitRepository::open`, matching
/// how the real app always opens a repo it didn't just write to.
fn build_linear_history(depth: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create temp dir for linear-history repo");
    let repo = git2::Repository::init(dir.path()).expect("git init");
    repo.set_head("refs/heads/main")
        .expect("point HEAD at main");

    let mut clock: i64 = 1_700_000_000;
    let mut last_oid: Option<git2::Oid> = None;

    for i in 0..depth {
        std::fs::write(dir.path().join("counter.txt"), i.to_string()).expect("write counter file");

        let mut index = repo.index().expect("open index");
        index
            .add_path(std::path::Path::new("counter.txt"))
            .expect("stage counter file");
        index.write().expect("write index");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");

        clock += 1;
        let sig = git2::Signature::new(
            "Test Author",
            "author@example.test",
            &git2::Time::new(clock, 0),
        )
        .expect("build signature");

        let parents: Vec<git2::Commit> = match last_oid {
            Some(oid) => vec![repo.find_commit(oid).expect("find previous commit")],
            None => Vec::new(),
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();

        let oid = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                &format!("chore: step {i}"),
                &tree,
                &parent_refs,
            )
            .expect("create commit");
        last_oid = Some(oid);
    }

    dir
}

/// Sanity check on the fixture builder itself: all five distinct oids show
/// up, and none are accidentally equal
#[test]
fn fixture_produces_five_distinct_commits() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let oids: HashSet<&str> = [
        fixture.root.as_str(),
        fixture.base.as_str(),
        fixture.main_tip.as_str(),
        fixture.feature.as_str(),
        fixture.merge.as_str(),
    ]
    .into_iter()
    .collect();
    assert_eq!(oids.len(), 5);

    let listed: HashSet<&str> = fixture
        .oids_newest_first
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(
        oids, listed,
        "the convenience list should match the named fields exactly"
    );
}

/// `README.md` is written once, by the root commit, and never again. Every
/// later commit carries it forward unchanged, so a naive "the path exists in
/// this tree" filter would return all five commits instead of one.
#[test]
fn file_history_returns_only_the_commits_that_changed_the_path() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let history = repo
        .file_history("README.md", WalkStart::Head, 100)
        .expect("walk the history of README.md");

    let oids: Vec<&str> = history.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(oids, vec![fixture.root.as_str()]);
    assert_eq!(history[0].summary, "chore: init repository");
    assert_eq!(
        history[0].parents.len(),
        0,
        "CommitInfo should carry real parents, not the filtered neighbours"
    );
}

/// `src/lib.rs` is added by `base`, edited by `main_tip`, and reverted by the
/// merge (which takes `feature`'s tree, where main's edit is absent). All
/// three differ from their first parent at that path; `feature` and `root` do
/// not. Order follows the walk, newest first.
#[test]
fn file_history_covers_adds_edits_and_first_parent_merges() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let history = repo
        .file_history("src/lib.rs", WalkStart::Head, 100)
        .expect("walk the history of src/lib.rs");

    let oids: Vec<&str> = history.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(
        oids,
        vec![
            fixture.merge.as_str(),
            fixture.main_tip.as_str(),
            fixture.base.as_str(),
        ]
    );
}

/// A file added on a side branch appears twice: once for the commit that
/// added it, and once for the merge, where it is new relative to the merge's
/// first parent. That is what `git log -- <path>` reports too.
#[test]
fn file_history_includes_a_side_branch_file_and_its_merge() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let history = repo
        .file_history("src/feature.rs", WalkStart::Head, 100)
        .expect("walk the history of src/feature.rs");

    let oids: Vec<&str> = history.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(oids, vec![fixture.merge.as_str(), fixture.feature.as_str()]);
}

/// `max_count` bounds matches, not walk steps: the walk keeps going past
/// non-matching commits until it has that many hits.
#[test]
fn file_history_stops_after_max_count_matches() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let history = repo
        .file_history("src/lib.rs", WalkStart::Head, 2)
        .expect("walk the history of src/lib.rs");

    let oids: Vec<&str> = history.iter().map(|c| c.oid.as_str()).collect();
    assert_eq!(
        oids,
        vec![fixture.merge.as_str(), fixture.main_tip.as_str()]
    );
}

/// A path no commit ever contained is empty rather than an error: the pane
/// reports "nothing touched this" instead of a libgit2 failure.
#[test]
fn file_history_of_an_unknown_path_is_empty() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let history = repo
        .file_history("does/not/exist.txt", WalkStart::Head, 100)
        .expect("an unknown path is not an error");
    assert!(history.is_empty());
}

/// `chmod +x` leaves the blob byte-identical, so a comparison of blob oids
/// alone would call the commit unchanged and drop it from the file's history —
/// plausibly the very commit the user opened the pane on. Git compares the mode
/// too, which is why `git log -- script.sh` lists it, and so does
/// `file_history`.
#[test]
fn file_history_includes_a_mode_only_change() {
    let (_repo_dir, fixture) = TestRepo::build_mode_change_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let history = repo
        .file_history("script.sh", WalkStart::Head, 100)
        .expect("walk the history of script.sh");

    let oids: Vec<&str> = history.iter().map(|c| c.oid.as_str()).collect();
    let expected = [fixture.chmod.as_str(), fixture.added.as_str()];
    assert_eq!(
        oids, expected,
        "the chmod commit should appear, newest first, above the commit that added the file"
    );
    assert!(
        !oids.contains(&fixture.unrelated.as_str()),
        "a commit that never touched script.sh should still be skipped"
    );
}

/// Guards the fixture, not the code: if the chmod commit ever stopped being
/// mode-only, the test above would pass without proving anything. Asserts the
/// blob is identical across the two commits and only the mode moved.
#[test]
fn the_mode_change_fixture_changes_only_the_mode() {
    let (_repo_dir, fixture) = TestRepo::build_mode_change_fixture();
    let repo = git2::Repository::open(&fixture.repo_path).expect("open fixture repo with git2");

    let entry_at = |commit_oid: &str| {
        let oid = git2::Oid::from_str(commit_oid).expect("parse commit oid");
        let commit = repo.find_commit(oid).expect("find fixture commit");
        let tree = commit.tree().expect("commit has a tree");
        let entry = tree
            .get_path(std::path::Path::new("script.sh"))
            .expect("script.sh is in the tree");
        (entry.id(), entry.filemode())
    };

    let (before_blob, before_mode) = entry_at(&fixture.added);
    let (after_blob, after_mode) = entry_at(&fixture.chmod);

    assert_eq!(
        before_blob, after_blob,
        "the chmod commit should not have touched the blob"
    );
    assert_eq!(before_mode, common::REGULAR);
    assert_eq!(after_mode, common::EXECUTABLE);
}

/// The filtered list feeds the same panes as full history, so it has to lay
/// out as graph rows. Its commits are not neighbours in the real graph, which
/// is why `build_linear` exists.
#[test]
fn file_history_lays_out_as_one_row_per_commit() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let history = repo
        .file_history("src/lib.rs", WalkStart::Head, 100)
        .expect("walk the history of src/lib.rs");
    let rows = GraphEngine::build_linear(&history);

    assert_eq!(rows.len(), history.len());
    for (row, commit) in rows.iter().zip(history.iter()) {
        assert_eq!(row.commit_oid, commit.oid);
        assert_eq!(row.node_lane, 0);
    }
}

/// A HEAD-only walk cannot see a commit that only exists on a branch nobody
/// has merged or checked out. `commits` (HEAD-only) should miss `stray_tip`;
#[test]
fn head_only_walk_misses_an_unmerged_branchs_commits() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let head_only = repo.commits(100).expect("walk HEAD-only history");
    let head_only_oids: HashSet<&str> = head_only.iter().map(|c| c.oid.as_str()).collect();

    assert!(
        head_only_oids.contains(fixture.root.as_str()),
        "root is an ancestor of HEAD (main) and should still be there"
    );
    assert!(
        head_only_oids.contains(fixture.main_tip.as_str()),
        "HEAD's own tip should be there"
    );
    assert!(
        !head_only_oids.contains(fixture.stray_tip.as_str()),
        "this is the bug: a HEAD-only walk should NOT reach a commit that \
         only exists on an unmerged branch"
    );
}

/// The fix: `commits_all_branches_with_progress` should surface exactly the
/// commit the HEAD-only walk above misses, in addition to everything HEAD
/// already reaches, deduplicated (the shared `root` commit appears once).
#[test]
fn all_branches_walk_finds_an_unmerged_branchs_commits() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let all = repo
        .commits_all_branches_with_progress(100, |_| true)
        .expect("walk all-branches history");
    let all_oids: Vec<&str> = all.iter().map(|c| c.oid.as_str()).collect();
    let all_oids_set: HashSet<&str> = all_oids.iter().copied().collect();

    assert!(
        all_oids_set.contains(fixture.stray_tip.as_str()),
        "the fix: an all-branches walk should reach `stray`'s tip"
    );
    assert!(all_oids_set.contains(fixture.root.as_str()));
    assert!(all_oids_set.contains(fixture.main_tip.as_str()));

    assert_eq!(
        all_oids.len(),
        all_oids_set.len(),
        "root is reachable from both main and stray; it must be yielded once, \
         not once per branch that reaches it"
    );
    assert_eq!(
        all_oids_set.len(),
        3,
        "exactly root, main_tip and stray_tip should exist in this fixture"
    );
}

/// The all-branches walk is a superset of the HEAD-only one: everything
/// the default view already shows is still there once branches are added,
/// nothing HEAD reaches gets dropped by widening the walk.
#[test]
fn all_branches_walk_is_a_superset_of_the_head_only_walk() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let head_only: HashSet<String> = repo
        .commits(100)
        .expect("walk HEAD-only history")
        .into_iter()
        .map(|c| c.oid)
        .collect();
    let all_branches: HashSet<String> = repo
        .commits_all_branches_with_progress(100, |_| true)
        .expect("walk all-branches history")
        .into_iter()
        .map(|c| c.oid)
        .collect();

    assert!(
        head_only.is_subset(&all_branches),
        "every commit HEAD-only shows must still show once branches are included"
    );
}

/// `AppState::toggle_all_branches_history` should widen the graph pane's
/// commit list on the first press, and `close_history` should put the
/// original HEAD-only list back exactly, selection included.
#[test]
fn toggling_all_branches_widens_then_restores_head_only_history() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let mut state = AppState::new(Some(fixture.repo_path.clone()), false);

    // Wait for the app's own background load (spawned by `AppState::new`) to
    // land, so `state.commits` reflects the real HEAD-only startup load
    // rather than an empty list still in flight.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.commits.is_empty() {
        state.poll_load();
        assert!(
            std::time::Instant::now() < deadline,
            "startup load never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let head_only_len = state.commits.len();
    assert!(
        !state.commits.iter().any(|c| c.oid == fixture.stray_tip),
        "sanity check on the fixture: the HEAD-only startup load must not \
         already include the unmerged branch's commit"
    );

    state.toggle_all_branches_history();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !state.commits.iter().any(|c| c.oid == fixture.stray_tip) {
        state.poll_detail();
        assert!(
            std::time::Instant::now() < deadline,
            "all-branches request never resolved"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(
        state.history,
        gitloom::app::HistoryScope::AllBranches,
        "the graph pane should report itself as scoped to all branches"
    );
    let all_branches = repo
        .commits_all_branches_with_progress(100, |_| true)
        .expect("walk all-branches history for comparison");
    assert_eq!(
        state.commits.len(),
        all_branches.len(),
        "the widened list should match a direct all-branches walk"
    );

    let closed = state.close_history();
    assert!(closed, "close_history should report that it closed a scope");
    assert_eq!(
        state.commits.len(),
        head_only_len,
        "closing should put back exactly the original HEAD-only list"
    );
    assert!(
        !state.commits.iter().any(|c| c.oid == fixture.stray_tip),
        "the unmerged branch's commit should be gone again after closing"
    );
    assert_eq!(state.history, gitloom::app::HistoryScope::Head);
}

/// `--all` must widen the *startup* load itself: when the first load lands
/// the graph is already scoped to all branches and already contains the
/// unmerged branch's commit, with no second walk swapping content in after
/// the fact.
#[test]
fn the_all_flag_scopes_the_startup_load() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let mut state = AppState::new(Some(fixture.repo_path.clone()), true);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.commits.is_empty() {
        state.poll_load();
        assert!(
            std::time::Instant::now() < deadline,
            "startup load never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(
        state.history,
        gitloom::app::HistoryScope::AllBranches,
        "the graph should open scoped to all branches"
    );
    assert!(
        state.commits.iter().any(|c| c.oid == fixture.stray_tip),
        "the very first load must already include the unmerged branch's commit"
    );
}

/// Under `--all` the view parked when a scope opens is the all-branches
/// history, not HEAD: opening a file's history and closing it again must
/// land back on all branches, with `history` matching what is on screen.
#[test]
fn an_all_branches_start_returns_to_all_branches_after_a_file_scope() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let mut state = AppState::new(Some(fixture.repo_path.clone()), true);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.commits.is_empty() {
        state.poll_load();
        assert!(
            std::time::Instant::now() < deadline,
            "startup load never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Enter a file scope the way the files pane would: stage a selected path
    // (`changed_files` and its list state are plain fields) and ask for its
    // history, then wait for the scope to land.
    state.changed_files = vec!["src/lib.rs".to_string()];
    state.files_list_state.select(Some(0));
    state.open_file_history();

    let wanted = gitloom::app::HistoryScope::File {
        path: "src/lib.rs".to_string(),
        start: WalkStart::HeadAndLocalBranches,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.history != wanted {
        state.poll_detail();
        assert!(
            std::time::Instant::now() < deadline,
            "file-history request never resolved"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert!(
        state.close_history(),
        "closing the file scope should restore the parked view"
    );
    assert_eq!(
        state.history,
        gitloom::app::HistoryScope::AllBranches,
        "the restored scope is what was parked, not unconditionally HEAD"
    );
    assert!(
        state.commits.iter().any(|c| c.oid == fixture.stray_tip),
        "the restored view is the all-branches history, not HEAD-only"
    );
}

/// The walk start is not decoration for `file_history`: a path that only
/// exists on an unmerged branch is invisible to a HEAD-rooted walk and
/// found by one rooted at the local branch tips.
#[test]
fn file_history_honors_the_walk_start() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");

    let head_only = repo
        .file_history("stray.rs", WalkStart::Head, 100)
        .expect("walk stray.rs history from HEAD");
    assert!(
        head_only.is_empty(),
        "the bug: a HEAD-rooted walk cannot see a file only an unmerged \
         branch ever touched"
    );

    let all_branches = repo
        .file_history("stray.rs", WalkStart::HeadAndLocalBranches, 100)
        .expect("walk stray.rs history from the branch tips");
    assert_eq!(
        all_branches
            .iter()
            .map(|c| c.oid.as_str())
            .collect::<Vec<_>>(),
        vec![fixture.stray_tip.as_str()],
        "the fix: the commit that introduced stray.rs is exactly the stray tip"
    );
}

/// From the all-branches graph, pressing `l` on a path
/// that only an unmerged branch touched must open a real history for it.
#[test]
fn file_history_from_the_all_branches_graph_walks_branch_tips() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let mut state = AppState::new(Some(fixture.repo_path.clone()), true);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.commits.is_empty() {
        state.poll_load();
        assert!(
            std::time::Instant::now() < deadline,
            "startup load never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    state.changed_files = vec!["stray.rs".to_string()];
    state.files_list_state.select(Some(0));
    state.open_file_history();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !matches!(
        state.history,
        gitloom::app::HistoryScope::File { ref path, .. } if path == "stray.rs"
    ) {
        state.poll_detail();
        assert!(
            std::time::Instant::now() < deadline,
            "file-history request never resolved"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert!(
        state.commits.iter().any(|c| c.oid == fixture.stray_tip),
        "the file history should contain the commit that touched stray.rs"
    );
    assert!(
        !state
            .status
            .as_ref()
            .is_some_and(|s| s.is_error() && s.text().contains("No commits")),
        "no dead-end error may be surfaced for a path the all-branches \
         graph just showed a commit for"
    );
}

/// Under `--all` the first `a` used to be swallowed (nothing was
/// parked, `close_history` returned false, the method returned). It must
/// fetch the HEAD-only history instead, and the press after that must put
/// the all-branches view back.
#[test]
fn an_all_session_first_a_press_fetches_head_history() {
    let (_repo_dir, fixture) = TestRepo::build_unmerged_branch_fixture();
    let mut state = AppState::new(Some(fixture.repo_path.clone()), true);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.commits.is_empty() {
        state.poll_load();
        assert!(
            std::time::Instant::now() < deadline,
            "startup load never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // First press: leaves the all-branches base for the HEAD-only view.
    state.toggle_all_branches_history();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.history != gitloom::app::HistoryScope::Head {
        state.poll_detail();
        assert!(
            std::time::Instant::now() < deadline,
            "the HEAD-history request never resolved: `a` was swallowed"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        !state.commits.iter().any(|c| c.oid == fixture.stray_tip),
        "the HEAD-only view must not contain the unmerged branch's commit"
    );

    // Second press: restores the parked all-branches view synchronously,
    // without another request to wait for.
    state.toggle_all_branches_history();
    assert_eq!(
        state.history,
        gitloom::app::HistoryScope::AllBranches,
        "the second press restores the all-branches base"
    );
    assert!(
        state.commits.iter().any(|c| c.oid == fixture.stray_tip),
        "the restored view is the all-branches history the app opened with"
    );
}
