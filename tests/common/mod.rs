//! Builds a small, real repository on disk with `git2` so the git layer,
//! search, minimap, and graph parent-handling can be exercised against
//! actual commit/tree/ref objects instead of only hand-built `CommitInfo`s.
//!
//! Timestamps and the author/committer identity are fixed so tests are
//! deterministic and don't depend on the machine's git config.
#![allow(dead_code)]

use git2::{Repository, Signature};
use std::path::Path;
use tempfile::TempDir;

/// A temp repository under construction. The `TempDir` is kept alive on the
/// returned `Fixture`'s owner; the directory and its contents are removed
/// when this struct drops, so callers must hold onto it for the test's
/// duration (see `build_standard_fixture`, which returns both).
pub struct TestRepo {
    dir: TempDir,
    repo: Repository,
    /// Base seconds for deterministic, strictly-increasing commit timestamps.
    clock: i64,
}

/// Oids of the fixture's notable commits, named for what the caller will
/// want to assert against rather than by position in history.
pub struct Fixture {
    pub repo_path: std::path::PathBuf,
    /// First commit on `main`.
    pub root: String,
    /// Second commit on `main`, branch point for `feature`.
    pub base: String,
    /// Third, tip-of-`main`-before-merge commit ("fix: handle empty input").
    pub main_tip: String,
    /// The one commit on `feature`.
    pub feature: String,
    /// Merge commit bringing `feature` into `main`; two parents.
    pub merge: String,
    /// All five oids above. `TOPOLOGICAL | TIME` from `main`'s tip is only
    /// guaranteed to put `merge` first and `root` last (children before
    /// parents); the exact order of `main_tip` vs. `feature` in between is
    /// an implementation detail of libgit2's tie-breaking, so tests should
    /// only rely on the first/last guarantee and on all five being present.
    pub oids_newest_first: Vec<String>,
}

/// Git's tree mode for an executable blob, written as it appears in
/// `git ls-tree` output.
pub const EXECUTABLE: i32 = 0o100_755;

/// Git's tree mode for an ordinary, non-executable blob.
pub const REGULAR: i32 = 0o100_644;

/// Oids of the mode-change fixture, built by
/// [`TestRepo::build_mode_change_fixture`].
pub struct ModeFixture {
    pub repo_path: std::path::PathBuf,
    /// Root commit; adds `script.sh` as a regular file.
    pub added: String,
    /// Adds `README.md` and leaves `script.sh` alone, so a file-history walk
    /// has a commit it is expected to skip.
    pub unrelated: String,
    /// Marks `script.sh` executable without changing a byte of it.
    pub chmod: String,
}

/// Oids of the unmerged-branch fixture, built by
/// [`TestRepo::build_unmerged_branch_fixture`].
pub struct UnmergedFixture {
    pub repo_path: std::path::PathBuf,
    /// Root commit on `main`; also where `stray` branches off.
    pub root: String,
    /// Tip of `main`. `HEAD` stays checked out here, never on `stray`, so a
    /// HEAD-only walk cannot see `stray_tip` by accident.
    pub main_tip: String,
    /// Tip of `stray`, a branch never merged into `main`.
    pub stray_tip: String,
}

impl TestRepo {
    pub fn init() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir for test repo");
        let repo = Repository::init(dir.path()).expect("git init temp repo");
        // `init.defaultBranch` varies by machine/git config; pin it explicitly
        // so the fixture's topology doesn't depend on the test runner's
        // global git config.
        repo.set_head("refs/heads/main")
            .expect("point HEAD at refs/heads/main before the first commit");
        Self {
            dir,
            repo,
            clock: 1_700_000_000, // arbitrary fixed epoch; only ordering matters
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// A long, purely linear history: `depth` commits, each touching the
    /// same file, with no branches or merges.
    ///
    /// Exists for two different reasons depending on who calls it: the
    /// `#[ignore]`d revwalk benchmark in `tests/git_repository.rs` needs a
    /// history deep enough to make an up-front full-history cost visible,
    /// and the pagination tests in `tests/pagination.rs` need one deep
    /// enough to force more than one page. Both only care about depth and
    /// topology, not content, so one builder serves both rather than each
    /// keeping its own copy.
    ///
    /// Deliberately bypasses `commit_file`: that goes through
    /// `self.repo.index()` and re-reads/re-writes the whole index on every
    /// call, which is fine for the handful of commits every other fixture
    /// here makes but turns quadratic at a few thousand. This writes the one
    /// file directly into the object database with `Repository::blob` and
    /// touches only that one entry through a `TreeBuilder`, so the cost per
    /// commit stays flat as `depth` grows.
    pub fn build_long_history_fixture(depth: usize) -> (Self, Vec<String>) {
        let mut repo = Self::init();
        let mut oids = Vec::with_capacity(depth);

        for i in 0..depth {
            let ts = repo.next_timestamp();
            let sig = Self::signature_at(ts);

            let blob_oid = repo
                .repo
                .blob(i.to_string().as_bytes())
                .expect("write counter blob");
            let parent_commits: Vec<git2::Commit> = match repo.repo.head() {
                Ok(head) => vec![head.peel_to_commit().expect("HEAD peels to a commit")],
                Err(_) => Vec::new(), // first commit in the repo: no parent
            };
            let parent_tree = parent_commits
                .first()
                .map(|commit| commit.tree().expect("previous commit has a tree"));
            let mut builder = repo
                .repo
                .treebuilder(parent_tree.as_ref())
                .expect("open treebuilder");
            builder
                .insert("counter.txt", blob_oid, REGULAR)
                .expect("insert counter file into tree");
            let tree_oid = builder.write().expect("write tree");
            let tree = repo.repo.find_tree(tree_oid).expect("find written tree");

            let parent_refs: Vec<&git2::Commit> = parent_commits.iter().collect();
            let oid = repo
                .repo
                .commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    &format!("chore: step {i}"),
                    &tree,
                    &parent_refs,
                )
                .expect("create commit");
            oids.push(oid.to_string());
        }

        (repo, oids)
    }

    fn next_timestamp(&mut self) -> i64 {
        // Strictly increasing so TIME-ordered sort has no ties to break
        // arbitrarily, and so `commit_details`'s date formatting has a
        // stable value to check. Only touches `self.clock`, never
        // `self.repo`, so callers can hold a borrow from `self.repo` (a
        // Tree, Commit, or Object) across this call without conflict.
        self.clock += 60;
        self.clock
    }

    fn signature_at(timestamp: i64) -> Signature<'static> {
        Signature::new(
            "Test Author",
            "author@example.test",
            &git2::Time::new(timestamp, 0),
        )
        .expect("build deterministic signature")
    }

    /// Writes `contents` to `relative_path` inside the worktree, stages it,
    /// and commits on `HEAD` with `parents`. Returns the new commit's oid
    /// (full hex string, matching what `CommitInfo::oid` stores).
    fn commit_file(&mut self, relative_path: &str, contents: &str, summary: &str) -> String {
        let full_path = self.dir.path().join(relative_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs for fixture file");
        }
        std::fs::write(&full_path, contents).expect("write fixture file");

        let ts = self.next_timestamp();

        let mut index = self.repo.index().expect("open repo index");
        index
            .add_path(Path::new(relative_path))
            .expect("stage fixture file");
        index.write().expect("write index");
        let tree_oid = index.write_tree().expect("write tree from index");
        let tree = self.repo.find_tree(tree_oid).expect("find written tree");

        let sig = Self::signature_at(ts);
        let parent_commits: Vec<git2::Commit> = match self.repo.head() {
            Ok(head) => vec![head.peel_to_commit().expect("HEAD peels to a commit")],
            Err(_) => Vec::new(), // first commit in the repo: no parent
        };
        let parent_refs: Vec<&git2::Commit> = parent_commits.iter().collect();

        let oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, summary, &tree, &parent_refs)
            .expect("create commit");
        oid.to_string()
    }

    /// Commits `contents` at `relative_path` onto `refname` (e.g.
    /// `"refs/heads/stray"`) directly, without checking that branch out or
    /// touching `HEAD` or the working tree at all.
    ///
    /// `relative_path` must be a top-level name, for the same reason as
    /// `commit_mode`: `TreeBuilder` edits one tree, and no fixture needs a
    /// nested path here yet.
    ///
    /// Unlike `commit_file`, the new tree is built from `refname`'s current
    /// tip rather than through `self.repo.index()`: the index reflects
    /// whatever is checked out, so writing through it here would silently
    /// commit onto the wrong branch (or corrupt the real working tree, since
    /// nothing was actually checked out to `refname`). `Repository::blob`
    /// writes the content straight into the object database, and
    /// `TreeBuilder` adds it to a tree built from `refname`'s tip, exactly
    /// mirroring how `commit_mode` edits a tree without touching disk.
    fn commit_on_ref(
        &mut self,
        refname: &str,
        relative_path: &str,
        contents: &str,
        summary: &str,
    ) -> String {
        assert!(
            !relative_path.contains('/'),
            "commit_on_ref only handles top-level paths, got `{relative_path}`"
        );
        let ts = self.next_timestamp();

        let reference = self
            .repo
            .find_reference(refname)
            .unwrap_or_else(|e| panic!("find_reference({refname}) failed: {e}"));
        let parent_commit = reference
            .peel_to_commit()
            .expect("target ref peels to a commit");
        let parent_tree = parent_commit.tree().expect("target ref's tip has a tree");

        let blob_oid = self
            .repo
            .blob(contents.as_bytes())
            .expect("write blob content");
        let mut builder = self
            .repo
            .treebuilder(Some(&parent_tree))
            .expect("open a treebuilder on the target ref's tree");
        builder
            .insert(relative_path, blob_oid, REGULAR)
            .expect("insert the new file into the tree");
        let tree_oid = builder.write().expect("write the rebuilt tree");
        let tree = self.repo.find_tree(tree_oid).expect("find rebuilt tree");

        let sig = Self::signature_at(ts);
        let oid = self
            .repo
            .commit(Some(refname), &sig, &sig, summary, &tree, &[&parent_commit])
            .expect("create commit on target ref");
        oid.to_string()
    }

    /// Commits on `HEAD` with two explicit parents (a merge commit), taking
    /// `feature`'s tree so the merge is trivially resolvable without needing
    /// a real three-way merge for the fixture's purposes.
    fn commit_merge(&mut self, other_branch: &str, summary: &str) -> String {
        let ts = self.next_timestamp();

        let head_commit = self
            .repo
            .head()
            .expect("HEAD exists before merging")
            .peel_to_commit()
            .expect("HEAD peels to a commit");
        let other_commit = self
            .repo
            .find_branch(other_branch, git2::BranchType::Local)
            .expect("find branch to merge")
            .get()
            .peel_to_commit()
            .expect("branch tip peels to a commit");

        let tree = other_commit.tree().expect("branch tip has a tree");
        let sig = Self::signature_at(ts);
        let oid = self
            .repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                summary,
                &tree,
                &[&head_commit, &other_commit],
            )
            .expect("create merge commit");
        oid.to_string()
    }

    /// Commits a change to `relative_path`'s file mode, leaving its content
    /// byte-identical, and returns the new commit's oid.
    ///
    /// The mode is set by rebuilding HEAD's tree rather than by touching the
    /// worktree, because Windows has no executable bit: `index.add_path` reads
    /// the mode off the filesystem there and would always produce `100644`, so
    /// a fixture built that way would quietly become a no-op commit and any
    /// test using it would pass for the wrong reason.
    ///
    /// `relative_path` must be a top-level name. `TreeBuilder` edits one tree,
    /// so a nested path would need each parent tree rebuilt in turn, and no
    /// fixture needs that yet.
    fn commit_mode(&mut self, relative_path: &str, mode: i32, summary: &str) -> String {
        assert!(
            !relative_path.contains('/'),
            "commit_mode only handles top-level paths, got `{relative_path}`"
        );
        // Before any borrow of `self.repo`: `next_timestamp` takes `&mut self`.
        let ts = self.next_timestamp();

        let head_commit = self
            .repo
            .head()
            .expect("HEAD exists before a mode change")
            .peel_to_commit()
            .expect("HEAD peels to a commit");
        let tree = head_commit.tree().expect("HEAD has a tree");
        let entry = tree
            .get_path(Path::new(relative_path))
            .expect("the path being chmod-ed is already in HEAD's tree");

        let mut builder = self
            .repo
            .treebuilder(Some(&tree))
            .expect("open a treebuilder on HEAD's tree");
        // Same oid, new mode: re-inserting replaces the entry in place.
        builder
            .insert(relative_path, entry.id(), mode)
            .expect("re-insert the entry with a new mode");
        let tree_oid = builder.write().expect("write the rebuilt tree");
        let new_tree = self.repo.find_tree(tree_oid).expect("find rebuilt tree");

        let sig = Self::signature_at(ts);
        let oid = self
            .repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                summary,
                &new_tree,
                &[&head_commit],
            )
            .expect("create mode-change commit");
        oid.to_string()
    }

    fn create_branch_at(&mut self, name: &str, target_oid: &str) {
        let oid = git2::Oid::from_str(target_oid).expect("parse oid for branch target");
        let commit = self
            .repo
            .find_commit(oid)
            .expect("find branch target commit");
        self.repo
            .branch(name, &commit, false)
            .expect("create branch");
    }

    fn checkout_branch(&mut self, name: &str) {
        let refname = format!("refs/heads/{name}");
        self.repo.set_head(&refname).expect("set HEAD to branch");
        self.repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .expect("checkout branch");
    }

    fn tag(&mut self, name: &str, target_oid: &str) {
        let ts = self.next_timestamp();
        let oid = git2::Oid::from_str(target_oid).expect("parse oid for tag target");
        let object = self.repo.find_object(oid, None).expect("find tag target");
        let sig = Self::signature_at(ts);
        self.repo
            .tag(name, &object, &sig, "release marker", false)
            .expect("create tag");
    }

    /// Builds the standard topology used by the integration tests:
    pub fn build_standard_fixture() -> (Self, Fixture) {
        let mut repo = Self::init();

        let root = repo.commit_file("README.md", "# Test Repo\n", "chore: init repository");
        repo.tag("v0.1.0", &root);

        let base = repo.commit_file(
            "src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
            "feat: add add() helper",
        );

        repo.create_branch_at("feature", &base);

        let main_tip = repo.commit_file(
            "src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n// handle empty input on main\n",
            "fix: handle empty input",
        );

        repo.checkout_branch("feature");
        let feature = repo.commit_file(
            "src/feature.rs",
            "pub fn greet() -> &'static str {\n    \"hello\"\n}\n",
            "feat: add greet() on feature branch",
        );

        repo.checkout_branch("main");
        let merge = repo.commit_merge("feature", "merge: bring in feature branch");

        let fixture = Fixture {
            repo_path: repo.path().to_path_buf(),
            oids_newest_first: vec![
                merge.clone(),
                feature.clone(),
                main_tip.clone(),
                base.clone(),
                root.clone(),
            ],
            root,
            base,
            main_tip,
            feature,
            merge,
        };
        (repo, fixture)
    }

    /// A history in which one commit changes only a file's mode.
    ///
    /// Deliberately separate from [`TestRepo::build_standard_fixture`], whose
    /// tests assert exact commit lists: adding a commit there would have meant
    /// rewriting assertions that have nothing to do with file modes.
    ///
    /// Ordered so the chmod lands last. `commit_file` writes its tree from the
    /// index, which knows nothing about a mode set through a `TreeBuilder`, so a
    /// later `commit_file` would silently reset the mode to `100644` and turn
    /// this fixture into *two* mode changes.
    pub fn build_mode_change_fixture() -> (Self, ModeFixture) {
        let mut repo = Self::init();

        let added = repo.commit_file("script.sh", "#!/bin/sh\necho hi\n", "feat: add script");
        let unrelated = repo.commit_file("README.md", "# Test Repo\n", "docs: add a readme");
        let chmod = repo.commit_mode("script.sh", EXECUTABLE, "chore: make script executable");

        let fixture = ModeFixture {
            repo_path: repo.path().to_path_buf(),
            added,
            unrelated,
            chmod,
        };
        (repo, fixture)
    }

    /// A history with a branch that is never merged back and never checked
    /// out again after being created: `HEAD` stays on `main` the whole time.
    pub fn build_unmerged_branch_fixture() -> (Self, UnmergedFixture) {
        let mut repo = Self::init();

        let root = repo.commit_file("README.md", "# Test Repo\n", "chore: init repository");
        repo.create_branch_at("stray", &root);

        let main_tip = repo.commit_file(
            "src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
            "feat: add add() helper on main",
        );

        // A commit made directly on `stray` without ever checking it out:
        // `commit_file` commits on whatever `HEAD` currently resolves to, and
        // a non-HEAD branch ref can still be advanced by passing its full
        // ref name instead of `"HEAD"`.
        let stray_tip = repo.commit_on_ref(
            "refs/heads/stray",
            "stray.rs",
            "pub fn only_on_stray() {}\n",
            "feat: work in progress on a branch nobody merged",
        );

        let fixture = UnmergedFixture {
            repo_path: repo.path().to_path_buf(),
            root,
            main_tip,
            stray_tip,
        };
        (repo, fixture)
    }
}
