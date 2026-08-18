use crate::git::repository::{GitRepository, RepoInfo};
use std::path::PathBuf;

pub enum RepoState {
    None,
    Error(String),
    Loaded(GitRepository, RepoInfo),
}

pub struct AppState {
    pub quit: bool,
    pub repo_state: RepoState,
}

impl AppState {
    pub fn new(path: Option<PathBuf>) -> Self {
        let search_path =
            path.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let repo_state = match GitRepository::open(&search_path) {
            Ok(repo) => {
                let info = repo.info();
                RepoState::Loaded(repo, info)
            }
            Err(e) => RepoState::Error(e.message().to_string()),
        };

        Self {
            quit: false,
            repo_state,
        }
    }
}
