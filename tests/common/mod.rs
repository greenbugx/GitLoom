//! Builds a small, real repository on disk with `git2` so the git layer,
//! search, minimap, and graph parent-handling can be exercised against
//! actual commit/tree/ref objects instead of only hand-built `CommitInfo`s.
//!
//! Timestamps and the author/committer identity are fixed so tests are
//! deterministic and don't depend on the machine's git config.

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
}
