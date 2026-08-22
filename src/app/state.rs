use crate::app::loading::{self, LoadMessage, LoadingState};
use crate::git::commit::CommitInfo;
use crate::git::repository::{GitRepository, RefBadge, RepoInfo};
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::git::commit::CommitDetails;
use crate::graph::layout::GraphRow;

pub enum RepoState {
    None,
    Error(String),
    Loading(GitRepository),
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
    pub search_query: String,
    pub is_searching: bool,
    pub search_results: Vec<usize>,
    pub search_index: usize,
    /// Point-in-time OID -> ref badges snapshot, loaded once at init.
    /// `GitRepository::ref_map` is NOT live and must be rebuilt if a
    /// future refresh/reload command is added.
    pub ref_map: HashMap<String, Vec<RefBadge>>,
    /// Precomputed minimap sparkline char per commit (indexed by commit order).
    /// Arrives together with the commits, so it is only empty without history.
    pub minimap: Vec<char>,
    /// Progress of the background load; `None` when nothing is loading.
    pub loading: Option<LoadingState>,
    /// Progress channel from the loading thread, dropped when the load ends.
    load_rx: Option<Receiver<LoadMessage>>,
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

        let (repo_state, loading, load_rx) = match GitRepository::open(&search_path) {
            Ok(repo) => {
                let git_dir = repo.path();
                let load_rx = loading::spawn(git_dir.clone());
                let label = repo_label(&git_dir, &search_path);
                (
                    RepoState::Loading(repo),
                    Some(LoadingState::new(label)),
                    Some(load_rx),
                )
            }
            Err(e) => (RepoState::Error(e.message().to_string()), None, None),
        };

        Self {
            quit: false,
            repo_state,
            commits: Vec::new(),
            graph_rows: Vec::new(),
            list_state: ListState::default(),
            commit_details: None,
            details_scroll: 0,
            details_scroll_x: 0,
            view_mode: ViewMode::Graph,
            changed_files: Vec::new(),
            diff_lines: Vec::new(),
            refs: None,
            refs_list_state: ListState::default(),
            search_query: String::new(),
            is_searching: false,
            search_results: Vec::new(),
            search_index: 0,
            ref_map: HashMap::new(),
            minimap: Vec::new(),
            loading,
            load_rx,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    pub fn tick(&mut self) {
        if let Some(load) = &mut self.loading {
            load.tick();
        }
    }

    pub fn poll_load(&mut self) {
        let Some(rx) = &self.load_rx else {
            return;
        };

        let mut messages = Vec::new();
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(message) => messages.push(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }

        for message in messages {
            self.apply(message);
        }

        if finished {
            self.load_rx = None;
            self.loading = None;
            if matches!(self.repo_state, RepoState::Loading(_)) {
                self.repo_state = RepoState::Error("failed to read repository".to_string());
            }
        }
    }

    fn apply(&mut self, message: LoadMessage) {
        match message {
            LoadMessage::Stage(stage) => {
                if let Some(load) = &mut self.loading {
                    load.set_stage(stage);
                }
            }
            LoadMessage::Progress { done, total } => {
                if let Some(load) = &mut self.loading {
                    load.done = done;
                    load.total = total;
                }
            }
            LoadMessage::Ready(data) => {
                let data = *data;
                self.commits = data.commits;
                self.graph_rows = data.graph_rows;
                self.ref_map = data.ref_map;
                self.minimap = data.minimap;
                if !self.commits.is_empty() {
                    self.list_state.select(Some(0));
                }
                let previous = std::mem::replace(&mut self.repo_state, RepoState::None);
                self.repo_state = match previous {
                    RepoState::Loading(repo) => RepoState::Loaded(repo, data.info),
                    other => other,
                };
                self.loading = None;
            }
            LoadMessage::Failed(err) => {
                self.repo_state = RepoState::Error(err);
                self.loading = None;
                self.load_rx = None;
            }
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

    /// Content dimensions of the details pane for the current view mode, as
    /// `(number_of_lines, widest_line_width)`.
    //  Used by both vertical and horizontal detail scrolling
    /// so the two view-mode match arms are not duplicated.
    fn content_dimensions(&self) -> (u16, u16) {
        match self.view_mode {
            ViewMode::Details => {
                if let Some(details) = &self.commit_details {
                    let lines = 30u16;
                    let mut max_len = 0usize;
                    max_len = max_len.max(details.summary.len());
                    max_len = max_len.max(details.author.len());
                    max_len = max_len.max(details.oid.len() + 2);
                    (lines, max_len as u16)
                } else {
                    (0, 0)
                }
            }
            ViewMode::Files => (
                self.changed_files.len() as u16,
                self.changed_files
                    .iter()
                    .map(|s| s.len())
                    .max()
                    .unwrap_or(0) as u16,
            ),
            ViewMode::Diff => (
                self.diff_lines.len() as u16,
                self.diff_lines.iter().map(|s| s.len()).max().unwrap_or(0) as u16,
            ),
            ViewMode::Graph | ViewMode::Refs => (0, 0),
        }
    }

    pub fn scroll_details_down(&mut self) {
        let (max_content_lines, _) = self.content_dimensions();
        let term_height = crossterm::terminal::size().map(|s| s.1).unwrap_or(24);
        let visible_height = term_height.saturating_sub(8);
        let max_scroll = max_content_lines.saturating_sub(visible_height);

        self.details_scroll = self.details_scroll.saturating_add(1).min(max_scroll);
    }

    pub fn scroll_details_up(&mut self) {
        self.details_scroll = self.details_scroll.saturating_sub(1);
    }

    pub fn scroll_details_right(&mut self) {
        let (_, max_line_len) = self.content_dimensions();

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
            ViewMode::Refs => {} // Reloading refs not needed on every commit change
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
            let total =
                refs.local_branches.len() + refs.remote_branches.len() + refs.tags.len() + 3;
            if total == 0 {
                return;
            }
            let i = match self.refs_list_state.selected() {
                Some(i) => {
                    if i >= total - 1 {
                        total - 1
                    } else {
                        i + 1
                    }
                }
                None => 0,
            };
            self.refs_list_state.select(Some(i));
        }
    }

    pub fn scroll_refs_up(&mut self) {
        if let Some(refs) = &self.refs {
            let total =
                refs.local_branches.len() + refs.remote_branches.len() + refs.tags.len() + 3;
            if total == 0 {
                return;
            }
            let i = match self.refs_list_state.selected() {
                Some(i) => {
                    if i == 0 {
                        0
                    } else {
                        i - 1
                    }
                }
                None => 0,
            };
            self.refs_list_state.select(Some(i));
        }
    }

    pub fn start_search(&mut self) {
        self.is_searching = true;
        self.search_query.clear();
    }

    pub fn execute_search(&mut self) {
        self.is_searching = false;
        if self.search_query.is_empty() {
            self.search_results.clear();
            return;
        }

        let q = self.search_query.to_lowercase();
        self.search_results.clear();
        for (i, commit) in self.commits.iter().enumerate() {
            if commit.summary.to_lowercase().contains(&q)
                || commit.author.to_lowercase().contains(&q)
                || commit.oid.to_lowercase().contains(&q)
            {
                self.search_results.push(i);
            }
        }

        self.search_index = 0;
        self.jump_to_current_search_result();
    }

    pub fn next_search_result(&mut self) {
        if self.search_results.is_empty() {
            return;
        }
        self.search_index = (self.search_index + 1) % self.search_results.len();
        self.jump_to_current_search_result();
    }

    pub fn previous_search_result(&mut self) {
        if self.search_results.is_empty() {
            return;
        }
        if self.search_index == 0 {
            self.search_index = self.search_results.len() - 1;
        } else {
            self.search_index -= 1;
        }
        self.jump_to_current_search_result();
    }

    fn jump_to_current_search_result(&mut self) {
        if let Some(&idx) = self.search_results.get(self.search_index) {
            self.list_state.select(Some(idx));
            self.refresh_view();
        }
    }
}

fn repo_label(git_dir: &Path, search_path: &Path) -> String {
    git_dir
        .parent()
        .and_then(|parent| parent.file_name())
        .or_else(|| git_dir.file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| search_path.display().to_string())
}
