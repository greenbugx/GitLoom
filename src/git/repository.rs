use git2::{Repository, StatusOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

const PROGRESS_INTERVAL: usize = 32;

const PARALLEL_POLL: Duration = Duration::from_millis(16);
const MIN_CHUNK: usize = 24;
const MAX_WORKERS: usize = 8;

pub struct GitRepository {
    repo: Repository,
}

pub struct RepoInfo {
    pub name: String,
    pub branch: String,
    pub is_clean: bool,
    pub is_bare: bool,
}

impl GitRepository {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, git2::Error> {
        let repo = Repository::discover(path)?;
        Ok(Self { repo })
    }

    pub fn path(&self) -> PathBuf {
        self.repo.path().to_path_buf()
    }

    pub fn info(&self) -> RepoInfo {
        let is_bare = self.repo.is_bare();

        let name = if is_bare {
            self.repo
                .path()
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        } else {
            self.repo
                .workdir()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        };

        let branch = if let Ok(head) = self.repo.head() {
            if head.is_branch() {
                head.shorthand().unwrap_or("unknown").to_string()
            } else {
                "Detached HEAD".to_string()
            }
        } else {
            "No HEAD".to_string()
        };

        let mut is_clean = true;
        if !is_bare {
            let mut opts = StatusOptions::new();
            opts.include_untracked(true).recurse_untracked_dirs(false);
            if let Ok(statuses) = self.repo.statuses(Some(&mut opts)) {
                is_clean = statuses.is_empty();
            }
        }

        RepoInfo {
            name,
            branch,
            is_clean,
            is_bare,
        }
    }

    pub fn commits(
        &self,
        max_count: usize,
    ) -> Result<Vec<crate::git::commit::CommitInfo>, git2::Error> {
        self.commits_with_progress(max_count, |_| true)
    }

    pub fn commits_with_progress<F>(
        &self,
        max_count: usize,
        mut on_progress: F,
    ) -> Result<Vec<crate::git::commit::CommitInfo>, git2::Error>
    where
        F: FnMut(usize) -> bool,
    {
        let mut revwalk = self.repo.revwalk()?;
        revwalk.push_head()?;
        revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let mut commits = Vec::new();
        for oid in revwalk.take(max_count) {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;

            let parents = commit.parent_ids().map(|id| id.to_string()).collect();
            let author = commit.author().name().unwrap_or("unknown").to_string();
            let timestamp = commit.time().seconds();
            let summary = commit.summary().unwrap_or(None).unwrap_or("").to_string();
            let message = commit.message().unwrap_or("").to_string();

            commits.push(crate::git::commit::CommitInfo {
                oid: oid.to_string(),
                parents,
                author,
                timestamp,
                summary,
                message,
            });

            if commits.len().is_multiple_of(PROGRESS_INTERVAL) && !on_progress(commits.len()) {
                return Ok(commits);
            }
        }
        on_progress(commits.len());
        Ok(commits)
    }

    pub fn commit_details(
        &self,
        oid_str: &str,
    ) -> Result<crate::git::commit::CommitDetails, git2::Error> {
        let oid = git2::Oid::from_str(oid_str)?;
        let commit = self.repo.find_commit(oid)?;

        let author = commit.author().name().unwrap_or("unknown").to_string();
        let time = commit.time();
        let parents: Vec<String> = commit.parent_ids().map(|id| id.to_string()).collect();
        let summary = commit.summary().unwrap_or(None).unwrap_or("").to_string();
        let message = commit.message().unwrap_or("").to_string();

        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };

        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        let stats = diff.stats()?;

        Ok(crate::git::commit::CommitDetails {
            oid: oid_str.to_string(),
            summary,
            message,
            author,
            date: time.seconds(),
            parents,
            files_changed: stats.files_changed(),
            insertions: stats.insertions(),
            deletions: stats.deletions(),
        })
    }

    pub fn changed_files(&self, oid_str: &str) -> Result<Vec<String>, git2::Error> {
        let oid = git2::Oid::from_str(oid_str)?;
        let commit = self.repo.find_commit(oid)?;
        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };

        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        let mut files = Vec::new();
        diff.print(git2::DiffFormat::NameOnly, |delta, _, _| {
            if let Some(path) = delta.new_file().path() {
                files.push(path.to_string_lossy().to_string());
            }
            true
        })?;
        Ok(files)
    }

    pub fn commit_diff(&self, oid_str: &str) -> Result<Vec<String>, git2::Error> {
        let oid = git2::Oid::from_str(oid_str)?;
        let commit = self.repo.find_commit(oid)?;
        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };

        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        let mut lines = Vec::new();
        diff.print(git2::DiffFormat::Patch, |_, _, line| {
            let origin = line.origin();
            let content = String::from_utf8_lossy(line.content())
                .trim_end_matches(&['\n', '\r'][..])
                .to_string();
            match origin {
                '+' | '-' | ' ' => lines.push(format!("{}{}", origin, content)),
                _ => lines.push(content),
            }
            true
        })?;
        Ok(lines)
    }
}

/// Note this is deliberately *not* called `Tag`: a tag is one kind of ref, but
/// a ref name is the name of any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefName(String);

impl RefName {
    pub fn new(name: impl Into<String>) -> Self {
        RefName(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RefName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A branch, with its locality carried by the enum tag itself. Consumers learn
/// whether a branch is local or remote by matching the variant; there is
/// deliberately no separate `kind` field duplicating that information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Branch {
    Local(RefName),
    Remote(RefName),
}

impl Branch {
    /// The branch's name, whichever locality it has. Lets display code that
    /// doesn't care about local-vs-remote avoid matching just to read the name.
    pub fn name(&self) -> &RefName {
        match self {
            Branch::Local(name) | Branch::Remote(name) => name,
        }
    }
}

/// Every ref discovered in a repository. This is the Git domain model only:
/// flattening it into section headers and selectable rows is the UI's job, so
/// nothing here knows about panes, headers or selection indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRefs {
    pub branches: Vec<Branch>,
    pub tags: Vec<RefName>,
}

/// A single ref pointing at a commit, rendered as an inline badge beside that
/// commit's summary. Branch locality already lives in [`Branch`], so this
/// composes `Branch` rather than repeating a local/remote/tag discriminant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    Branch(Branch),
    Tag(RefName),
}

impl Ref {
    /// The ref's name, whichever kind of ref it is.
    pub fn name(&self) -> &RefName {
        match self {
            Ref::Branch(branch) => branch.name(),
            Ref::Tag(name) => name,
        }
    }
}

impl GitRepository {
    pub fn refs(&self) -> Result<RepoRefs, git2::Error> {
        let mut branches = Vec::new();

        for (b, _) in self.repo.branches(Some(git2::BranchType::Local))?.flatten() {
            if let Ok(Some(name)) = b.name() {
                branches.push(Branch::Local(RefName::new(name)));
            }
        }

        for (b, _) in self
            .repo
            .branches(Some(git2::BranchType::Remote))?
            .flatten()
        {
            if let Ok(Some(name)) = b.name() {
                branches.push(Branch::Remote(RefName::new(name)));
            }
        }

        let mut tags = Vec::new();
        let tag_names = self.repo.tag_names(None)?;
        for i in 0..tag_names.len() {
            if let Ok(Some(name)) = tag_names.get(i) {
                tags.push(RefName::new(name));
            }
        }

        Ok(RepoRefs { branches, tags })
    }

    /// Build a point-in-time map from commit OID to the refs pointing at it
    /// (HEAD, local branches, remote branches and tags).
    ///
    /// NOTE: This is a SNAPSHOT built once from `repo.refs()` at load time.
    ///  It is NOT live, if a future refresh/reload command is added, this map must be rebuilt.
    /// :(
    pub fn ref_map(&self) -> Result<HashMap<String, Vec<Ref>>, git2::Error> {
        let mut map: HashMap<String, Vec<Ref>> = HashMap::new();

        // HEAD: when it points at a branch, the branch itself is added below
        // via `references()`, so only emit a distinct badge when detached.
        if let Ok(head) = self.repo.head()
            && let Ok(commit) = head.peel_to_commit()
            && !head.is_branch()
        {
            map.entry(commit.id().to_string())
                .or_default()
                .push(Ref::Branch(Branch::Local(RefName::new("HEAD"))));
        }

        for reference in self.repo.references()? {
            let reference = reference?;
            let name = reference.name().unwrap_or("");
            // Classify by prefix and strip it in one step: each arm both names
            // the ref kind and yields the short display name, so the kind and
            // the stripping can't fall out of sync.
            let git_ref = if let Some(short) = name.strip_prefix("refs/heads/") {
                Some(Ref::Branch(Branch::Local(RefName::new(short))))
            } else if let Some(short) = name.strip_prefix("refs/remotes/") {
                Some(Ref::Branch(Branch::Remote(RefName::new(short))))
            } else {
                name.strip_prefix("refs/tags/")
                    .map(|short| Ref::Tag(RefName::new(short)))
            };
            if let Some(git_ref) = git_ref
                && let Ok(commit) = reference.peel_to_commit()
            {
                map.entry(commit.id().to_string())
                    .or_default()
                    .push(git_ref);
            }
        }

        Ok(map)
    }

    /// (insertions, deletions) per commit oid, aligned with `oids`.
    pub fn commit_deltas(&self, oids: &[String]) -> Vec<(usize, usize)> {
        self.commit_deltas_with_progress(oids, |_| true)
    }

    /// Single-threaded; the loader uses [`commit_deltas_parallel`] instead.
    /// Returning `false` from `on_progress` abandons the pass.
    pub fn commit_deltas_with_progress<F>(
        &self,
        oids: &[String],
        mut on_progress: F,
    ) -> Vec<(usize, usize)>
    where
        F: FnMut(usize) -> bool,
    {
        let mut deltas = Vec::with_capacity(oids.len());
        for oid in oids {
            deltas.push(self.commit_delta(oid).unwrap_or((0, 0)));
            if deltas.len().is_multiple_of(PROGRESS_INTERVAL) && !on_progress(deltas.len()) {
                return deltas;
            }
        }
        on_progress(deltas.len());
        deltas
    }

    /// Insertions and deletions of a single commit against its first parent.
    fn commit_delta(&self, oid_str: &str) -> Option<(usize, usize)> {
        let oid = git2::Oid::from_str(oid_str).ok()?;
        let commit = self.repo.find_commit(oid).ok()?;
        let tree = commit.tree().ok()?;
        let parent_tree = if commit.parent_count() > 0 {
            commit.parent(0).ok()?.tree().ok()
        } else {
            None
        };
        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .ok()?;
        let stats = diff.stats().ok()?;
        Some((stats.insertions(), stats.deletions()))
    }
}

/// (insertions, deletions) per oid, measured on up to [`MAX_WORKERS`] threads.
///
/// A `git2::Repository` cannot cross threads, so each worker opens `git_dir`
/// itself. `on_progress` runs on the calling thread; returning `false` cancels
/// the pass and the unmeasured commits come back as `(0, 0)`.
pub fn commit_deltas_parallel<F>(
    git_dir: &Path,
    oids: &[String],
    mut on_progress: F,
) -> Vec<(usize, usize)>
where
    F: FnMut(usize) -> bool,
{
    if oids.is_empty() {
        on_progress(0);
        return Vec::new();
    }

    let chunk_len = oids.len().div_ceil(worker_count(oids.len()));
    let chunks: Vec<&[String]> = oids.chunks(chunk_len).collect();
    let lengths: Vec<usize> = chunks.iter().map(|chunk| chunk.len()).collect();
    let measured = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);

    let parts: Vec<Vec<(usize, usize)>> = std::thread::scope(|scope| {
        let measured = &measured;
        let cancelled = &cancelled;
        let mut handles = Vec::with_capacity(chunks.len());
        for &chunk in &chunks {
            let handle = scope.spawn(move || chunk_deltas(git_dir, chunk, measured, cancelled));
            handles.push(handle);
        }

        loop {
            // Checked before reading the counter so the last report is complete.
            let done = handles.iter().all(|handle| handle.is_finished());
            if !on_progress(measured.load(Ordering::Relaxed)) {
                cancelled.store(true, Ordering::Relaxed);
                break;
            }
            if done {
                break;
            }
            std::thread::sleep(PARALLEL_POLL);
        }

        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_default())
            .collect()
    });

    stitch(parts, &lengths)
}

fn worker_count(commits: usize) -> usize {
    let available = std::thread::available_parallelism()
        .map(|threads| threads.get())
        .unwrap_or(1);
    let useful = commits.div_ceil(MIN_CHUNK);
    available.min(useful).clamp(1, MAX_WORKERS)
}

fn chunk_deltas(
    git_dir: &Path,
    oids: &[String],
    measured: &AtomicUsize,
    cancelled: &AtomicBool,
) -> Vec<(usize, usize)> {
    let Ok(repo) = GitRepository::open(git_dir) else {
        return Vec::new();
    };

    let mut deltas = Vec::with_capacity(oids.len());
    for oid in oids {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        deltas.push(repo.commit_delta(oid).unwrap_or((0, 0)));
        measured.fetch_add(1, Ordering::Relaxed);
    }
    deltas
}

/// Joins the chunks back in order, padding short ones so a cancelled worker
/// cannot shift later commits' sparklines.
fn stitch(parts: Vec<Vec<(usize, usize)>>, lengths: &[usize]) -> Vec<(usize, usize)> {
    let mut deltas = Vec::with_capacity(lengths.iter().sum());
    for (part, length) in parts.into_iter().zip(lengths) {
        let missing = length.saturating_sub(part.len());
        deltas.extend(part.into_iter().take(*length));
        deltas.extend(std::iter::repeat_n((0, 0), missing));
    }
    deltas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_count_stays_within_its_bounds() {
        for commits in [0, 1, MIN_CHUNK, MIN_CHUNK * 3, 10_000] {
            let workers = worker_count(commits);
            assert!(
                workers >= 1,
                "{commits} commits: always at least one worker"
            );
            assert!(
                workers <= MAX_WORKERS,
                "{commits} commits: never past the cap"
            );
        }
    }

    #[test]
    fn a_tiny_history_is_not_split_across_threads() {
        assert_eq!(worker_count(1), 1);
        assert_eq!(worker_count(MIN_CHUNK), 1);
        assert!(
            worker_count(MIN_CHUNK + 1) <= 2,
            "one more commit, one more worker"
        );
    }

    #[test]
    fn chunking_covers_every_commit_exactly_once() {
        let oids: Vec<String> = (0..100).map(|i| i.to_string()).collect();
        let chunk_len = oids.len().div_ceil(worker_count(oids.len()));
        let chunks: Vec<&[String]> = oids.chunks(chunk_len).collect();

        assert!(chunks.len() <= MAX_WORKERS);
        assert_eq!(chunks.iter().map(|c| c.len()).sum::<usize>(), oids.len());
    }

    #[test]
    fn stitching_keeps_the_chunks_in_order() {
        let parts = vec![vec![(1, 1), (2, 2)], vec![(3, 3)]];
        assert_eq!(stitch(parts, &[2, 1]), vec![(1, 1), (2, 2), (3, 3)]);
    }

    #[test]
    fn a_short_chunk_is_padded_rather_than_closed_up() {
        let parts = vec![vec![(9, 9)], vec![(3, 3), (4, 4)]];
        let deltas = stitch(parts, &[2, 2]);

        assert_eq!(deltas.len(), 4);
        assert_eq!(deltas[1], (0, 0), "the gap is padded");
        assert_eq!(
            deltas[2],
            (3, 3),
            "so the next chunk still starts at index 2"
        );
    }
}
