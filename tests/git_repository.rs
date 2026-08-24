//! Exercises `GitRepository`, the minimap, and search against a real
//! on-disk repository (built by `tests/common`) instead of hand-built
//! `CommitInfo`s. `tests/graph_layout.rs` already covers the graph engine's
//! pure topology math in isolation; these tests cover the seam where that
//! data actually comes from libgit2 and where `AppState` puts it together.

mod common;

use common::TestRepo;
use gitloom::app::AppState;
use gitloom::git::repository::{GitRepository, RefKind};
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
            .any(|b| b.kind == RefKind::Tag && b.name == "v0.1.0"),
        "root should carry the v0.1.0 tag badge"
    );

    let merge_badges = ref_map
        .get(&fixture.merge)
        .expect("merge is main's tip and should carry the main badge");
    assert!(
        merge_badges
            .iter()
            .any(|b| b.kind == RefKind::Local && b.name == "main"),
        "merge should carry the local `main` branch badge"
    );

    let feature_badges = ref_map
        .get(&fixture.feature)
        .expect("feature tip should carry the feature badge");
    assert!(
        feature_badges
            .iter()
            .any(|b| b.kind == RefKind::Local && b.name == "feature")
    );

    // base is an ancestor of both tips but isn't itself pointed at by any
    // ref, so it should carry no badges at all.
    assert!(
        !ref_map.contains_key(&fixture.base),
        "an ancestor commit with no ref pointing at it should have no badges"
    );
}

/// `refs()` should list the branch and tag names created by the fixture,
/// and `RepoRefs::rows()` should flatten them with a header per section.
#[test]
fn refs_lists_branches_and_tags_and_rows_flattens_them() {
    let (_repo_dir, fixture) = TestRepo::build_standard_fixture();
    let repo = GitRepository::open(&fixture.repo_path).expect("open fixture repo");
    let refs = repo.refs().expect("list refs");

    assert!(refs.local_branches.contains(&"main".to_string()));
    assert!(refs.local_branches.contains(&"feature".to_string()));
    assert!(refs.tags.contains(&"v0.1.0".to_string()));

    let rows = refs.rows();
    let header_count = rows.iter().filter(|r| r.is_header()).count();
    assert_eq!(header_count, 3, "Local Branches / Remote Branches / Tags");

    let entry_count = rows.iter().filter(|r| !r.is_header()).count();
    assert_eq!(
        entry_count,
        refs.local_branches.len() + refs.remote_branches.len() + refs.tags.len(),
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
    let mut state = AppState::new(Some(fixture.repo_path.clone()));
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
