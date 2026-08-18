use git2::{Repository, StatusOptions};
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
}
