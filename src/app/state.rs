use crate::git::commit::CommitInfo;
use crate::git::repository::{GitRepository, RepoInfo};
use ratatui::widgets::ListState;
use std::path::PathBuf;

use crate::graph::layout::{GraphEngine, GraphRow};

pub enum RepoState {
    None,
    Error(String),
    Loaded(GitRepository, RepoInfo),
}

pub struct AppState {
    pub quit: bool,
    pub repo_state: RepoState,
    pub commits: Vec<CommitInfo>,
    pub graph_rows: Vec<GraphRow>,
    pub list_state: ListState,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(None)
    }
}

impl AppState {
    pub fn new(path: Option<PathBuf>) -> Self {
        let search_path =
            path.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let mut commits = Vec::new();
        let mut graph_rows = Vec::new();
        let mut list_state = ListState::default();

        let repo_state = match GitRepository::open(&search_path) {
            Ok(repo) => {
                let info = repo.info();
                if let Ok(loaded_commits) = repo.commits(1000) {
                    commits = loaded_commits;
                    graph_rows = GraphEngine::build(&commits);

                    let max_width = graph_rows
                        .iter()
                        .map(|r| r.glyphs.chars().count())
                        .max()
                        .unwrap_or(0);
                    for row in &mut graph_rows {
                        let current_len = row.glyphs.chars().count();
                        if current_len < max_width {
                            row.glyphs.push_str(&" ".repeat(max_width - current_len));
                        }
                    }
                }
                if !commits.is_empty() {
                    list_state.select(Some(0));
                }
                RepoState::Loaded(repo, info)
            }
            Err(e) => RepoState::Error(e.message().to_string()),
        };

        Self {
            quit: false,
            repo_state,
            commits,
            graph_rows,
            list_state,
        }
    }

    pub fn next_commit(&mut self) {
        if self.commits.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.commits.len() - 1 {
                    self.commits.len() - 1
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn previous_commit(&mut self) {
        if self.commits.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }
}
