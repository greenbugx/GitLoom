use crate::app::loading::{self, LoadMessage, LoadingState};
use crate::git::commit::CommitInfo;
use crate::git::repository::{Branch, GitRepository, Ref, RefName, RepoInfo, RepoRefs};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use unicode_width::UnicodeWidthStr;

use crate::git::commit::CommitDetails;
use crate::graph::layout::GraphRow;

/// One row of the refs pane: a non-selectable section header, a branch entry or
/// a tag entry.
///
/// This is a presentation type, which is why it lives here and not in
/// `git::repository`: the Git layer models refs, and the application layer
/// decides that they are shown as three labelled sections. Rows own their
/// values rather than borrowing from a [`RepoRefs`] so `AppState` can hold the
/// flattened rows next to the refs they came from without a self-referential
/// borrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefPaneRow {
    Header(&'static str),
    Branch(Branch),
    Tag(RefName),
}

impl RefPaneRow {
    pub fn is_header(&self) -> bool {
        matches!(self, RefPaneRow::Header(_))
    }
}

/// Flatten discovered refs into pane rows: each section header followed by its
/// entries, in `Local Branches / Remote Branches / Tags` order. Empty sections
/// still get a header row so the pane layout stays stable.
///
/// Flattening once, at load time, keeps the selectable-row count in one place
/// instead of recomputing `local + remote + tags + 3` everywhere the pane is
/// scrolled or rendered.
pub fn ref_pane_rows(refs: &RepoRefs) -> Vec<RefPaneRow> {
    let mut rows = Vec::with_capacity(refs.branches.len() + refs.tags.len() + 3);
    rows.push(RefPaneRow::Header("Local Branches"));
    rows.extend(
        refs.branches
            .iter()
            .filter(|b| matches!(b, Branch::Local(_)))
            .cloned()
            .map(RefPaneRow::Branch),
    );
    rows.push(RefPaneRow::Header("Remote Branches"));
    rows.extend(
        refs.branches
            .iter()
            .filter(|b| matches!(b, Branch::Remote(_)))
            .cloned()
            .map(RefPaneRow::Branch),
    );
    rows.push(RefPaneRow::Header("Tags"));
    rows.extend(refs.tags.iter().cloned().map(RefPaneRow::Tag));
    rows
}

pub enum RepoState {
    None,
    Error(String),
    Loading(GitRepository),
    Loaded(GitRepository, RepoInfo),
}

#[derive(PartialEq, Clone, Copy)]
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
    pub refs: Option<RepoRefs>,
    /// `refs` flattened into header/entry rows, rebuilt whenever `refs` is.
    /// Scrolling and rendering both walk this instead of recomputing
    /// `local + remote + tags + 3` and re-deriving header positions.
    pub refs_rows: Vec<RefPaneRow>,
    pub refs_list_state: ListState,
    pub search_query: String,
    pub is_searching: bool,
    pub search_results: Vec<usize>,
    pub search_index: usize,
    /// Point-in-time OID -> ref badges snapshot, loaded once at init.
    /// `GitRepository::ref_map` is NOT live and must be rebuilt if a
    /// future refresh/reload command is added.
    pub ref_map: HashMap<String, Vec<Ref>>,
    /// Precomputed minimap sparkline char per commit (indexed by commit order).
    /// Arrives together with the commits, so it is only empty without history.
    pub minimap: Vec<char>,
    /// Progress of the background load; `None` when nothing is loading.
    pub loading: Option<LoadingState>,
    /// Progress channel from the loading thread, dropped when the load ends.
    load_rx: Option<Receiver<LoadMessage>>,
    /// Last-rendered inner area of the details/files/diff/refs pane, updated
    /// by `ui::render` every frame. Scroll clamping reads real geometry from
    /// here instead of re-deriving it from `crossterm::terminal::size()` and
    /// the layout percentages baked into `ui::mod`.
    pub details_pane: Rect,
    /// Short status/error message shown in the bottom bar.
    pub status: Option<String>,
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
            refs_rows: Vec::new(),
            refs_list_state: ListState::default(),
            search_query: String::new(),
            is_searching: false,
            search_results: Vec::new(),
            search_index: 0,
            ref_map: HashMap::new(),
            minimap: Vec::new(),
            loading,
            load_rx,
            details_pane: Rect::default(),
            status: None,
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
        step_selection(&mut self.list_state, self.commits.len(), 1);
    }

    pub fn previous_commit(&mut self) {
        if self.commits.is_empty() {
            return;
        }
        step_selection(&mut self.list_state, self.commits.len(), -1);
    }

    pub fn load_details(&mut self) {
        let Some(idx) = self.list_state.selected() else {
            return;
        };
        let Some(commit) = self.commits.get(idx) else {
            return;
        };
        let RepoState::Loaded(repo, _) = &self.repo_state else {
            self.status = Some("No repository loaded".to_string());
            return;
        };
        match repo.commit_details(&commit.oid) {
            Ok(details) => {
                self.commit_details = Some(details);
                self.view_mode = ViewMode::Details;
                self.details_scroll = 0;
                self.details_scroll_x = 0;
                self.status = None;
            }
            Err(e) => self.status = Some(format!("Failed to load commit details: {e}")),
        }
    }

    pub fn close_details(&mut self) {
        self.view_mode = ViewMode::Graph;
        self.status = None;
    }

    /// Content dimensions of the details pane for the current view mode, as
    /// `(number_of_lines, widest_line_display_width)`. Widths are measured in
    /// display columns (`unicode-width`), not bytes, so non-ASCII summaries,
    /// authors, and paths scroll to the correct offset instead of overshooting.
    //  Used by both vertical and horizontal detail scrolling
    /// so the two view-mode match arms are not duplicated.
    fn content_dimensions(&self) -> (u16, u16) {
        match self.view_mode {
            ViewMode::Details => {
                if let Some(details) = &self.commit_details {
                    // Same formatter `ui::render` uses to build the Details
                    // paragraph, so the line count can't drift from what's
                    // actually on screen.
                    let text = crate::app::details_text::format(details);
                    let lines = text.lines().count() as u16;
                    let max_len = text.lines().map(|l| l.width()).max().unwrap_or(0);
                    (lines, max_len as u16)
                } else {
                    (0, 0)
                }
            }
            ViewMode::Files => (
                self.changed_files.len() as u16,
                self.changed_files
                    .iter()
                    .map(|s| s.width())
                    .max()
                    .unwrap_or(0) as u16,
            ),
            ViewMode::Diff => (
                self.diff_lines.len() as u16,
                self.diff_lines.iter().map(|s| s.width()).max().unwrap_or(0) as u16,
            ),
            ViewMode::Graph | ViewMode::Refs => (0, 0),
        }
    }

    pub fn scroll_details_down(&mut self) {
        let (max_content_lines, _) = self.content_dimensions();
        // Inner height of the last-rendered details pane; the border already
        // accounts for itself since `details_pane` is the block's inner area.
        let visible_height = self.details_pane.height;
        let max_scroll = max_content_lines.saturating_sub(visible_height);

        self.details_scroll = self.details_scroll.saturating_add(1).min(max_scroll);
    }

    pub fn scroll_details_up(&mut self) {
        self.details_scroll = self.details_scroll.saturating_sub(1);
    }

    pub fn scroll_details_right(&mut self) {
        let (_, max_line_len) = self.content_dimensions();

        let visible_width = self.details_pane.width;
        let max_scroll_x = max_line_len.saturating_sub(visible_width);

        self.details_scroll_x = self.details_scroll_x.saturating_add(1).min(max_scroll_x);
    }

    pub fn scroll_details_left(&mut self) {
        self.details_scroll_x = self.details_scroll_x.saturating_sub(1);
    }

    pub fn load_files(&mut self) {
        let Some(idx) = self.list_state.selected() else {
            return;
        };
        let Some(commit) = self.commits.get(idx) else {
            return;
        };
        let RepoState::Loaded(repo, _) = &self.repo_state else {
            self.status = Some("No repository loaded".to_string());
            return;
        };
        match repo.changed_files(&commit.oid) {
            Ok(files) => {
                self.changed_files = files;
                self.view_mode = ViewMode::Files;
                self.details_scroll = 0;
                self.details_scroll_x = 0;
                self.status = None;
            }
            Err(e) => self.status = Some(format!("Failed to load changed files: {e}")),
        }
    }

    pub fn load_diff(&mut self) {
        let Some(idx) = self.list_state.selected() else {
            return;
        };
        let Some(commit) = self.commits.get(idx) else {
            return;
        };
        let RepoState::Loaded(repo, _) = &self.repo_state else {
            self.status = Some("No repository loaded".to_string());
            return;
        };
        match repo.commit_diff(&commit.oid) {
            Ok(lines) => {
                self.diff_lines = lines;
                self.view_mode = ViewMode::Diff;
                self.details_scroll = 0;
                self.details_scroll_x = 0;
                self.status = None;
            }
            Err(e) => self.status = Some(format!("Failed to load diff: {e}")),
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
        let RepoState::Loaded(repo, _) = &self.repo_state else {
            self.status = Some("No repository loaded".to_string());
            return;
        };
        match repo.refs() {
            Ok(refs) => {
                self.refs_rows = ref_pane_rows(&refs);
                self.refs = Some(refs);
                self.view_mode = ViewMode::Refs;
                // Land on the first selectable entry, not the "Local
                // Branches" header, if there is one.
                let first_entry = self.refs_rows.iter().position(|r| !r.is_header());
                self.refs_list_state.select(first_entry.or(Some(0)));
                self.status = None;
            }
            Err(e) => self.status = Some(format!("Failed to load branches & tags: {e}")),
        }
    }

    pub fn scroll_refs_down(&mut self) {
        step_selection_skipping(&mut self.refs_list_state, &self.refs_rows, 1, |r| {
            r.is_header()
        });
    }

    pub fn scroll_refs_up(&mut self) {
        step_selection_skipping(&mut self.refs_list_state, &self.refs_rows, -1, |r| {
            r.is_header()
        });
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

fn step_selection(state: &mut ListState, len: usize, step: i32) {
    if len == 0 {
        return;
    }
    let last = len - 1;
    let i = match state.selected() {
        Some(i) => {
            if step > 0 {
                if i >= last { last } else { i + 1 }
            } else if i == 0 {
                0
            } else {
                i - 1
            }
        }
        None => 0,
    };
    state.select(Some(i));
}

/// Same as `step_selection`, but skips over rows for which `is_unselectable`
/// returns true (the refs pane's section headers) instead of landing on them.
/// Falls back to the current index if every row in the stepped direction is
/// unselectable, so the selection never disappears.
fn step_selection_skipping<T>(
    state: &mut ListState,
    rows: &[T],
    step: i32,
    is_unselectable: impl Fn(&T) -> bool,
) {
    if rows.is_empty() {
        return;
    }
    let last = rows.len() - 1;
    let start = state.selected().unwrap_or(0).min(last);
    let mut i = start;
    loop {
        let next = if step > 0 {
            if i >= last {
                break;
            }
            i + 1
        } else {
            if i == 0 {
                break;
            }
            i - 1
        };
        i = next;
        if !is_unselectable(&rows[i]) {
            state.select(Some(i));
            return;
        }
    }
    // Hit the end without finding a selectable row; keep the original
    // selection instead of landing on a header.
    if !is_unselectable(&rows[start]) {
        state.select(Some(start));
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
