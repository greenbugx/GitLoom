use crate::git::commit::CommitInfo;
use crate::git::repository::{GitRepository, RepoInfo};
use ratatui::widgets::ListState;
use std::path::PathBuf;

use crate::git::commit::CommitDetails;
use crate::graph::layout::{GraphEngine, GraphRow};

pub enum RepoState {
    None,
    Error(String),
    Loaded(GitRepository, RepoInfo),
}

#[derive(PartialEq)]
pub enum ViewMode {
    Graph,
    Details,
    Files,
    Diff,
    Refs,
}

pub struct AppState {
    pub quit: bool,
    pub repo_state: RepoState,
    pub commits: Vec<CommitInfo>,
    pub graph_rows: Vec<GraphRow>,
    pub list_state: ListState,
    pub commit_details: Option<CommitDetails>,
    pub details_scroll: u16,
    pub details_scroll_x: u16,
    pub view_mode: ViewMode,
    pub changed_files: Vec<String>,
    pub diff_lines: Vec<String>,
    pub refs: Option<crate::git::repository::RepoRefs>,
    pub refs_list_state: ListState,
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
            commit_details: None,
            details_scroll: 0,
            details_scroll_x: 0,
            view_mode: ViewMode::Graph,
            changed_files: Vec::new(),
            diff_lines: Vec::new(),
            refs: None,
            refs_list_state: ListState::default(),
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

    pub fn load_details(&mut self) {
        if let Some(idx) = self.list_state.selected()
            && let Some(commit) = self.commits.get(idx)
            && let RepoState::Loaded(repo, _) = &self.repo_state
            && let Ok(details) = repo.commit_details(&commit.oid)
        {
            self.commit_details = Some(details);
            self.view_mode = ViewMode::Details;
            self.details_scroll = 0;
            self.details_scroll_x = 0;
        }
    }

    pub fn close_details(&mut self) {
        self.view_mode = ViewMode::Graph;
    }

    pub fn scroll_details_down(&mut self) {
        let max_content_lines: u16 = match self.view_mode {
            ViewMode::Details => 30,
            ViewMode::Files => self.changed_files.len() as u16,
            ViewMode::Diff => self.diff_lines.len() as u16,
            ViewMode::Graph => 0,
            ViewMode::Refs => 0,
        };
        let term_height = crossterm::terminal::size().map(|s| s.1).unwrap_or(24);
        let visible_height = term_height.saturating_sub(8);
        let max_scroll = max_content_lines.saturating_sub(visible_height);

        self.details_scroll = self.details_scroll.saturating_add(1).min(max_scroll);
    }

    pub fn scroll_details_up(&mut self) {
        self.details_scroll = self.details_scroll.saturating_sub(1);
    }

    pub fn scroll_details_right(&mut self) {
        let max_line_len: u16 = match self.view_mode {
            ViewMode::Details => {
                if let Some(details) = &self.commit_details {
                    let mut max_len = 0;
                    max_len = max_len.max(details.summary.len());
                    max_len = max_len.max(details.author.len());
                    max_len = max_len.max(details.oid.len() + 2);
                    max_len as u16
                } else {
                    0
                }
            },
            ViewMode::Files => self.changed_files.iter().map(|s| s.len()).max().unwrap_or(0) as u16,
            ViewMode::Diff => self.diff_lines.iter().map(|s| s.len()).max().unwrap_or(0) as u16,
            ViewMode::Graph => 0,
            ViewMode::Refs => 0,
        };

        let term_width = crossterm::terminal::size().map(|s| s.0).unwrap_or(80);
        let visible_width = (term_width * 30 / 100).saturating_sub(2);
        let max_scroll_x = max_line_len.saturating_sub(visible_width);

        self.details_scroll_x = self.details_scroll_x.saturating_add(1).min(max_scroll_x);
    }

    pub fn scroll_details_left(&mut self) {
        self.details_scroll_x = self.details_scroll_x.saturating_sub(1);
    }

    pub fn load_files(&mut self) {
        if let Some(idx) = self.list_state.selected()
            && let Some(commit) = self.commits.get(idx)
            && let RepoState::Loaded(repo, _) = &self.repo_state
            && let Ok(files) = repo.changed_files(&commit.oid)
        {
            self.changed_files = files;
            self.view_mode = ViewMode::Files;
            self.details_scroll = 0;
            self.details_scroll_x = 0;
        }
    }

    pub fn load_diff(&mut self) {
        if let Some(idx) = self.list_state.selected()
            && let Some(commit) = self.commits.get(idx)
            && let RepoState::Loaded(repo, _) = &self.repo_state
            && let Ok(lines) = repo.commit_diff(&commit.oid)
        {
            self.diff_lines = lines;
            self.view_mode = ViewMode::Diff;
            self.details_scroll = 0;
            self.details_scroll_x = 0;
        }
    }

    pub fn refresh_view(&mut self) {
        // Only refresh if not in Graph mode to preserve lazy loading
        match self.view_mode {
            ViewMode::Graph => {}
            ViewMode::Details => self.load_details(),
            ViewMode::Files => self.load_files(),
            ViewMode::Diff => self.load_diff(),
            ViewMode::Refs => {}, // Reloading refs not needed on every commit change
        }
    }

    pub fn load_refs(&mut self) {
        if let RepoState::Loaded(repo, _) = &self.repo_state
            && let Ok(refs) = repo.refs()
        {
            self.refs = Some(refs);
            self.view_mode = ViewMode::Refs;
            self.refs_list_state.select(Some(0));
        }
    }

    pub fn scroll_refs_down(&mut self) {
        if let Some(refs) = &self.refs {
            let total = refs.local_branches.len() + refs.remote_branches.len() + refs.tags.len() + 3;
            if total == 0 { return; }
            let i = match self.refs_list_state.selected() {
                Some(i) => if i >= total - 1 { total - 1 } else { i + 1 },
                None => 0,
            };
            self.refs_list_state.select(Some(i));
        }
    }

    pub fn scroll_refs_up(&mut self) {
        if let Some(refs) = &self.refs {
            let total = refs.local_branches.len() + refs.remote_branches.len() + refs.tags.len() + 3;
            if total == 0 { return; }
            let i = match self.refs_list_state.selected() {
                Some(i) => if i == 0 { 0 } else { i - 1 },
                None => 0,
            };
            self.refs_list_state.select(Some(i));
        }
    }
}
