use git2::{Repository, StatusOptions};
use std::collections::HashMap;
use std::path::Path;

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
            opts.include_untracked(true).recurse_untracked_dirs(true);
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
        }
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

pub struct RepoRefs {
    pub local_branches: Vec<String>,
    pub remote_branches: Vec<String>,
    pub tags: Vec<String>,
}

/// The kind of a ref badge shown inline next to a commit summary
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

/// A single ref badge displayed next to a commit (branch name, remote branch or tag)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefBadge {
    pub kind: RefKind,
    pub name: String,
}

impl GitRepository {
    pub fn refs(&self) -> Result<RepoRefs, git2::Error> {
        let mut local_branches = Vec::new();
        let mut remote_branches = Vec::new();

        for (b, _) in self.repo.branches(Some(git2::BranchType::Local))?.flatten() {
            if let Ok(Some(name)) = b.name() {
                local_branches.push(name.to_string());
            }
        }

        for (b, _) in self
            .repo
            .branches(Some(git2::BranchType::Remote))?
            .flatten()
        {
            if let Ok(Some(name)) = b.name() {
                remote_branches.push(name.to_string());
            }
        }

        let mut tags = Vec::new();
        let tag_names = self.repo.tag_names(None)?;
        for i in 0..tag_names.len() {
            if let Ok(Some(name)) = tag_names.get(i) {
                tags.push(name.to_string());
            }
        }

        Ok(RepoRefs {
            local_branches,
            remote_branches,
            tags,
        })
    }

    /// Build a point-in-time map from commit OID to the refs pointing at it
    /// (HEAD, local branches, remote branches and tags).
    ///
    /// NOTE: This is a SNAPSHOT built once from `repo.refs()` at load time.
    ///  It is NOT live, if a future refresh/reload command is added, this map must be rebuilt.
    /// :(
    pub fn ref_map(&self) -> Result<HashMap<String, Vec<RefBadge>>, git2::Error> {
        let mut map: HashMap<String, Vec<RefBadge>> = HashMap::new();

        // HEAD: when it points at a branch, the branch itself is added below
        // via `references()`, so only emit a distinct badge when detached.
        if let Ok(head) = self.repo.head()
            && let Ok(commit) = head.peel_to_commit()
            && !head.is_branch()
        {
            map.entry(commit.id().to_string())
                .or_default()
                .push(RefBadge {
                    kind: RefKind::Local,
                    name: "HEAD".to_string(),
                });
        }

        for reference in self.repo.references()? {
            let reference = reference?;
            let name = reference.name().unwrap_or("");
            let kind = if name.starts_with("refs/heads/") {
                Some(RefKind::Local)
            } else if name.starts_with("refs/remotes/") {
                Some(RefKind::Remote)
            } else if name.starts_with("refs/tags/") {
                Some(RefKind::Tag)
            } else {
                None
            };
            if let Some(kind) = kind
                && let Ok(commit) = reference.peel_to_commit()
            {
                let short = name
                    .strip_prefix("refs/heads/")
                    .or_else(|| name.strip_prefix("refs/remotes/"))
                    .or_else(|| name.strip_prefix("refs/tags/"))
                    .unwrap_or(name);
                map.entry(commit.id().to_string())
                    .or_default()
                    .push(RefBadge {
                        kind,
                        name: short.to_string(),
                    });
            }
        }

        Ok(map)
    }

    /// Compute the (insertions, deletions) pair for every commit, aligned with
    /// the order of `commits`. Used to precompute minimap sparkline values once
    /// at load time. A commit that fails to diff falls back to `(0, 0)`.
    pub fn commit_deltas(&self, commits: &[crate::git::commit::CommitInfo]) -> Vec<(usize, usize)> {
        commits
            .iter()
            .map(|c| {
                let oid = git2::Oid::from_str(&c.oid).ok()?;
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
            })
            .map(|v| v.unwrap_or((0, 0)))
            .collect()
    }
}
