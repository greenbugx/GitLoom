use crate::app::detail::{DetailWorker, Payload, Request};
use crate::app::loading::{self, LoadMessage, LoadingState};
use crate::git::commit::CommitInfo;
use crate::git::repository::{Branch, GitRepository, Ref, RefName, RepoInfo, RepoRefs, WalkStart};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use unicode_width::UnicodeWidthStr;

use crate::git::commit::CommitDetails;
use crate::graph::layout::{GraphEngine, GraphRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryScope {
    /// Everything reachable from HEAD, as loaded at startup. Default.
    Head,
    /// Everything reachable from HEAD or any local branch tip: the `--all`
    /// startup flag, or [`AppState::toggle_all_branches_history`].
    AllBranches,
    /// Only the commits that touched this path, filtered from the walk
    /// `start` names. A file's history is a subset of the graph it was
    /// opened from, so the graph it came from is part of the scope: from
    /// the all-branches view it walks the branch tips too, or a path that
    /// only exists on an unmerged branch reports "no commits" even though
    /// the selected commit just changed it.
    File { path: String, start: WalkStart },
}

impl HistoryScope {
    /// The revwalk starting points that produce this scope's commits.
    fn walk_start(&self) -> WalkStart {
        match self {
            HistoryScope::Head => WalkStart::Head,
            HistoryScope::AllBranches => WalkStart::HeadAndLocalBranches,
            HistoryScope::File { start, .. } => *start,
        }
    }
}

/// A one-line message for the bottom bar.
///
/// The two cases are distinguished because the bar used to be error-only and
/// painted red unconditionally; now that a successful `y` reports through the
/// same field, "sent to clipboard" in red would read as a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Something worked, and said so.
    Notice(String),
    /// Something failed, in a way a key press would otherwise have swallowed. :(
    Error(String),
}

impl Status {
    pub fn text(&self) -> &str {
        match self {
            Status::Notice(text) | Status::Error(text) => text,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Status::Error(_))
    }
}

/// The view parked while a scope (a file's history, or all branches) is on
/// screen.
///
/// Scoping replaces the commit list rather than filtering a view of it,
/// because the graph rows, the minimap and the selection are all index
/// aligned with `commits` and would otherwise need to be recomputed on the
/// way back. Holding one copy costs memory only while scoped, and it is
/// dropped the moment the parked view is restored.
struct HistorySnapshot {
    /// The scope the parked view was showing: [`HistoryScope::Head`]
    /// normally, [`HistoryScope::AllBranches`] when the app started with
    /// `--all`. Restored by `close_history` so the pane title and the toggle
    /// state keep describing what is actually on screen.
    scope: HistoryScope,
    commits: Vec<CommitInfo>,
    graph_rows: Vec<GraphRow>,
    minimap: Vec<char>,
    selected: Option<usize>,
}

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
    /// Selection in the CHANGED FILES pane. The pane is a list rather than a
    /// paragraph so a path can be picked and its history opened with `l`.
    pub files_list_state: ListState,
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
    /// Detail/files/diff/file-history fetches, run off the UI thread. `None`
    /// only when no repository could be opened at all.
    detail: Option<DetailWorker>,
    /// Which commits the graph pane is currently showing.
    pub history: HistoryScope,
    /// The view parked while `history` is scoped to a file or to all
    /// branches: HEAD history normally, all-branches history when the app
    /// started with `--all`.
    saved_history: Option<Box<HistorySnapshot>>,
    /// Whether the `?` keymap overlay is up. Independent of `view_mode` so
    /// dismissing it returns to whatever pane was underneath.
    pub show_help: bool,
    /// Last-rendered inner area of the graph pane, recorded by `ui::render` so
    /// PageUp/PageDown move by the number of rows actually on screen.
    pub graph_pane: Rect,
    /// Last-rendered inner area of the details/files/diff/refs pane, updated
    /// by `ui::render` every frame. Scroll clamping reads real geometry from
    /// here instead of re-deriving it from `crossterm::terminal::size()` and
    /// the layout percentages baked into `ui::mod`.
    pub details_pane: Rect,
    /// Short message shown in the bottom bar, red for errors.
    pub status: Option<Status>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(None, false)
    }
}

impl AppState {
    /// `start_all_branches` mirrors the `--all` CLI flag: the initial
    /// background load then walks every local branch tip instead of HEAD only
    /// (see `loading::spawn`), and the graph opens with `history` already
    /// [`HistoryScope::AllBranches`].
    pub fn new(path: Option<PathBuf>, start_all_branches: bool) -> Self {
        let search_path =
            path.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let (repo_state, loading, load_rx, detail) = match GitRepository::open(&search_path) {
            Ok(repo) => {
                let git_dir = repo.path();
                let load_rx = loading::spawn(git_dir.clone(), start_all_branches);
                let label = repo_label(&git_dir, &search_path);
                (
                    RepoState::Loading(repo),
                    Some(LoadingState::new(label)),
                    Some(load_rx),
                    Some(DetailWorker::spawn(git_dir)),
                )
            }
            Err(e) => (RepoState::Error(e.message().to_string()), None, None, None),
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
            files_list_state: ListState::default(),
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
            detail,
            history: if start_all_branches {
                HistoryScope::AllBranches
            } else {
                HistoryScope::Head
            },
            saved_history: None,
            show_help: false,
            graph_pane: Rect::default(),
            details_pane: Rect::default(),
            status: None,
        }
    }

    /// True while anything is running that will change the screen without a
    /// keypress: the initial load, or an in-flight detail fetch. The main loop
    /// polls with a timeout while this holds instead of blocking on input, so
    /// results appear as they land.
    pub fn is_busy(&self) -> bool {
        self.is_loading() || self.detail.as_ref().is_some_and(DetailWorker::is_busy)
    }

    /// Placeholder text for a pane whose contents are still being fetched.
    pub fn pending_label(&self) -> Option<&'static str> {
        self.detail
            .as_ref()
            .and_then(DetailWorker::pending)
            .map(Request::pending_label)
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    fn notice(&mut self, message: impl Into<String>) {
        self.status = Some(Status::Notice(message.into()));
    }

    fn error(&mut self, message: impl Into<String>) {
        self.status = Some(Status::Error(message.into()));
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

    /// Move the commit selection and pull the open pane along with it, so a
    /// diff or file list can be walked commit by commit without closing it.
    /// Bound to `J`/`K`, leaving `j`/`k` to scroll the pane's own contents.
    pub fn step_commit_and_follow(&mut self, direction: i32) {
        if self.commits.is_empty() {
            return;
        }
        step_selection(&mut self.list_state, self.commits.len(), direction);
        self.refresh_view();
    }

    /// `g`: the top of whatever currently has focus.
    ///
    /// `g`, `G` and the page keys dispatch on the view mode here rather than
    /// through four key-table arms each, because they mean one thing ("the top
    /// of the focused pane") that every pane can answer. The graph arms don't
    /// refresh the open pane, for the same reason `j`/`k` don't: reaching them
    /// means the graph *is* the focused pane. `J`/`K` is the binding that moves
    /// the selection with a pane in tow.
    pub fn go_first(&mut self) {
        match self.view_mode {
            ViewMode::Graph => {
                if !self.commits.is_empty() {
                    self.list_state.select(Some(0));
                }
            }
            ViewMode::Refs => {
                let first = self.refs_rows.iter().position(|r| !r.is_header());
                if let Some(first) = first {
                    self.refs_list_state.select(Some(first));
                }
            }
            ViewMode::Files => {
                if !self.changed_files.is_empty() {
                    self.files_list_state.select(Some(0));
                }
            }
            ViewMode::Details | ViewMode::Diff => {
                self.details_scroll = 0;
                self.details_scroll_x = 0;
            }
        }
    }

    /// `G`: the bottom of whatever currently has focus.
    pub fn go_last(&mut self) {
        match self.view_mode {
            ViewMode::Graph => {
                if !self.commits.is_empty() {
                    self.list_state.select(Some(self.commits.len() - 1));
                }
            }
            ViewMode::Refs => {
                let last = self.refs_rows.iter().rposition(|r| !r.is_header());
                if let Some(last) = last {
                    self.refs_list_state.select(Some(last));
                }
            }
            ViewMode::Files => {
                if !self.changed_files.is_empty() {
                    self.files_list_state
                        .select(Some(self.changed_files.len() - 1));
                }
            }
            ViewMode::Details | ViewMode::Diff => {
                let (lines, _) = self.content_dimensions();
                self.details_scroll = lines.saturating_sub(self.details_pane.height);
            }
        }
    }

    /// PageDown (`direction > 0`) and PageUp: a screenful of the focused pane.
    pub fn page(&mut self, direction: i32) {
        match self.view_mode {
            ViewMode::Graph => {
                if self.commits.is_empty() {
                    return;
                }
                let page = page_size(self.graph_pane.height);
                let last = self.commits.len() - 1;
                let current = self.list_state.selected().unwrap_or(0);
                let target = if direction > 0 {
                    current.saturating_add(page).min(last)
                } else {
                    current.saturating_sub(page)
                };
                self.list_state.select(Some(target));
            }
            ViewMode::Refs => {
                // Stepping repeatedly reuses the header-skipping logic instead
                // of re-deriving which row a page jump lands on.
                for _ in 0..page_size(self.details_pane.height) {
                    step_selection_skipping(
                        &mut self.refs_list_state,
                        &self.refs_rows,
                        direction,
                        |r| r.is_header(),
                    );
                }
            }
            ViewMode::Files => {
                if self.changed_files.is_empty() {
                    return;
                }
                let page = page_size(self.details_pane.height);
                let last = self.changed_files.len() - 1;
                let current = self.files_list_state.selected().unwrap_or(0);
                let target = if direction > 0 {
                    current.saturating_add(page).min(last)
                } else {
                    current.saturating_sub(page)
                };
                self.files_list_state.select(Some(target));
            }
            ViewMode::Details | ViewMode::Diff => {
                let page = page_size(self.details_pane.height) as u16;
                if direction > 0 {
                    let (lines, _) = self.content_dimensions();
                    let max_scroll = lines.saturating_sub(self.details_pane.height);
                    self.details_scroll = self.details_scroll.saturating_add(page).min(max_scroll);
                } else {
                    self.details_scroll = self.details_scroll.saturating_sub(page);
                }
            }
        }
    }

    pub fn next_file(&mut self) {
        step_selection(&mut self.files_list_state, self.changed_files.len(), 1);
    }

    pub fn previous_file(&mut self) {
        step_selection(&mut self.files_list_state, self.changed_files.len(), -1);
    }

    /// Ask the terminal to copy the selected commit's full OID.
    ///
    /// The status message says "sent to clipboard" rather than "copied"
    /// deliberately: OSC 52 gives no acknowledgement, so a terminal that
    /// ignores it would make a flat "copied" a lie. See [`crate::clipboard`].
    pub fn yank_selected_oid(&mut self) {
        let Some(oid) = self.selected_oid() else {
            self.error("No commit selected");
            return;
        };
        match crate::clipboard::copy(&oid) {
            Ok(()) => {
                let short = &oid[..7.min(oid.len())];
                self.notice(format!("{short} sent to clipboard"));
            }
            Err(e) => self.error(format!("Failed to write to the terminal: {e}")),
        }
    }

    /// The selected commit's oid, cloned so callers don't hold a borrow of
    /// `self` while mutating other fields.
    fn selected_oid(&self) -> Option<String> {
        let index = self.list_state.selected()?;
        Some(self.commits.get(index)?.oid.clone())
    }

    /// Hand `request` to the detail thread, reporting rather than silently
    /// doing nothing if there is nothing to hand it to.
    fn request(&mut self, request: Request) -> bool {
        let Some(detail) = &mut self.detail else {
            self.error("No repository loaded");
            return false;
        };
        if !detail.request(request) {
            self.error("Detail worker stopped");
            return false;
        }
        self.status = None;
        true
    }

    /// Apply whatever the detail thread has finished, if anything. Called from
    /// the main loop next to [`AppState::poll_load`].
    pub fn poll_detail(&mut self) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        let Some(result) = detail.poll() else {
            return;
        };
        match result {
            Ok(payload) => self.apply_payload(payload),
            Err(err) => self.error(err),
        }
    }

    fn apply_payload(&mut self, payload: Payload) {
        match payload {
            Payload::Details(details) => self.commit_details = Some(*details),
            Payload::Files(files) => {
                self.files_list_state
                    .select((!files.is_empty()).then_some(0));
                self.changed_files = files;
            }
            Payload::Diff(lines) => self.diff_lines = lines,
            Payload::FileHistory {
                path,
                start,
                commits,
            } => self.enter_file_history(path, start, commits),
            Payload::AllBranchesHistory(commits) => self.enter_all_branches_history(commits),
            Payload::HeadHistory(commits) => self.enter_head_history(commits),
        }
    }

    /// Request the selected commit's details and switch to the Details pane.
    ///
    /// The previous commit's content is dropped rather than left on screen
    /// under the new commit's heading; the pane shows a placeholder until the
    /// fetch lands. All three of these used to call libgit2 inline, which is
    /// what made a large commit freeze the terminal.
    pub fn load_details(&mut self) {
        let Some(oid) = self.selected_oid() else {
            return;
        };
        if self.request(Request::Details(oid)) {
            self.view_mode = ViewMode::Details;
            self.commit_details = None;
            self.details_scroll = 0;
            self.details_scroll_x = 0;
        }
    }

    pub fn close_details(&mut self) {
        self.view_mode = ViewMode::Graph;
        self.status = None;
        // A fetch still in flight would otherwise repaint a pane the user has
        // just closed.
        if let Some(detail) = &mut self.detail {
            detail.cancel();
        }
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
            ViewMode::Diff => (
                self.diff_lines.len() as u16,
                self.diff_lines.iter().map(|s| s.width()).max().unwrap_or(0) as u16,
            ),
            // The graph, refs and changed-files panes are lists: they track a
            // selection and let `List` handle its own offset, so there is no
            // scroll offset here to clamp.
            ViewMode::Graph | ViewMode::Refs | ViewMode::Files => (0, 0),
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
        let Some(oid) = self.selected_oid() else {
            return;
        };
        if self.request(Request::Files(oid)) {
            self.view_mode = ViewMode::Files;
            self.changed_files = Vec::new();
            self.files_list_state.select(None);
            self.details_scroll = 0;
            self.details_scroll_x = 0;
        }
    }

    pub fn load_diff(&mut self) {
        let Some(oid) = self.selected_oid() else {
            return;
        };
        if self.request(Request::Diff(oid)) {
            self.view_mode = ViewMode::Diff;
            self.diff_lines = Vec::new();
            self.details_scroll = 0;
            self.details_scroll_x = 0;
        }
    }

    /// Scope the graph pane to the history of the file selected in the CHANGED
    /// FILES pane. Bound to `l` there, where there is no horizontal scrolling
    /// to conflict with.
    ///
    /// The walk starts from the graph currently on screen (see
    /// [`HistoryScope::walk_start`]), not unconditionally from HEAD: from the
    /// all-branches view, a path that only exists on an unmerged branch would
    /// otherwise report "no commits" even though the selected commit just
    /// changed it.
    pub fn open_file_history(&mut self) {
        let Some(path) = self
            .files_list_state
            .selected()
            .and_then(|index| self.changed_files.get(index))
            .cloned()
        else {
            self.error("No file selected");
            return;
        };
        self.request(Request::FileHistory {
            path,
            start: self.history.walk_start(),
            max_count: loading::COMMIT_LIMIT,
        });
    }

    /// Toggle the graph pane between HEAD-only and every local branch. Bound
    /// to `a` in the graph pane, which never dead-ends: from either base
    /// (HEAD by default, all branches under `--all`) the first press walks
    /// the other view and parks the base, and from a scoped view the press
    /// closes back to whatever is parked underneath.
    ///
    /// The parked-underneath case is what keeps repeated presses cheap: the
    /// default session's second `a` restores the parked HEAD view without
    /// re-walking it, and an `--all` session's second `a` restores the parked
    /// all-branches view the same way.
    ///
    /// See [`crate::git::repository::GitRepository::commits_all_branches_with_progress`]
    /// for why this is local branches only, not `git log --all`'s full set.
    pub fn toggle_all_branches_history(&mut self) {
        // `a` always toggles to the other of the two graph views; a file
        // scope counts as whichever view it was opened from, so `a` switches
        // to all branches from there.
        let (target, request) = if self.history == HistoryScope::AllBranches {
            (
                HistoryScope::Head,
                Request::HeadHistory(loading::COMMIT_LIMIT),
            )
        } else {
            (
                HistoryScope::AllBranches,
                Request::AllBranchesHistory(loading::COMMIT_LIMIT),
            )
        };
        // The target may be the view already parked underneath this one
        // (HEAD under all-branches, or the reverse under `--all`): closing
        // restores it as it was, selection and minimap included, with no
        // walk at all.
        if self
            .saved_history
            .as_ref()
            .is_some_and(|saved| saved.scope == target)
            && self.close_history()
        {
            return;
        }
        self.request(request);
    }

    /// Swap the graph pane over to a single file's history, parking whatever
    /// graph it was opened from so `close_history` can put it back.
    fn enter_file_history(&mut self, path: String, start: WalkStart, commits: Vec<CommitInfo>) {
        if commits.is_empty() {
            self.error(format!("No commits in this history touched {path}"));
            return;
        }
        // Linear rows, not `GraphEngine::build`: see `build_linear` for why a
        // filtered list cannot be laid out as a graph.
        let graph_rows = GraphEngine::build_linear(&commits);
        self.enter_scoped_history(HistoryScope::File { path, start }, commits, graph_rows);
    }

    /// Swap the graph pane over to the all-branches history, parking
    /// whatever was underneath (HEAD history, or itself under `--all`) so
    /// `close_history` can put it back.
    fn enter_all_branches_history(&mut self, commits: Vec<CommitInfo>) {
        if commits.is_empty() {
            self.error("No commits in this repository");
            return;
        }
        // Real topology, unlike file history: parents are still exactly the
        // commit's real parents (nothing was filtered out commit-by-commit),
        // so the ordinary graph layout applies, merges and all.
        let graph_rows = GraphEngine::build(&commits);
        self.enter_scoped_history(HistoryScope::AllBranches, commits, graph_rows);
    }

    /// Swap the graph pane over to the HEAD-only history: the `a` toggle's
    /// other direction, reachable when the all-branches view is the base
    /// (an `--all` session). Parks the all-branches view underneath, so the
    /// next `a` (or `Esc`) restores it rather than re-walking it.
    fn enter_head_history(&mut self, commits: Vec<CommitInfo>) {
        if commits.is_empty() {
            self.error("No commits reachable from HEAD");
            return;
        }
        let graph_rows = GraphEngine::build(&commits);
        self.enter_scoped_history(HistoryScope::Head, commits, graph_rows);
    }

    /// Shared by the three `enter_*_history` methods: park the view
    /// underneath the first scope opened, then swap the graph pane's commit
    /// list and layout over to `commits`/`graph_rows`.
    fn enter_scoped_history(
        &mut self,
        scope: HistoryScope,
        commits: Vec<CommitInfo>,
        graph_rows: Vec<GraphRow>,
    ) {
        // Only the first scoping saves a snapshot. Jumping from one scoped
        // view straight to another (file history to all-branches, or from
        // one file to another) must not overwrite it with an already-scoped
        // list, or closing would land on the previous scope rather than on
        // the view the app opened with.
        if self.saved_history.is_none() {
            self.saved_history = Some(Box::new(HistorySnapshot {
                scope: self.history.clone(),
                commits: std::mem::take(&mut self.commits),
                graph_rows: std::mem::take(&mut self.graph_rows),
                minimap: std::mem::take(&mut self.minimap),
                selected: self.list_state.selected(),
            }));
        }

        self.graph_rows = graph_rows;
        self.commits = commits;
        // The minimap is deliberately empty while scoped. Commit sizes come
        // from the background size pass over the full list, and re-running it
        // for a subset would mean another walk for a decoration; `ui::render`
        // already treats a missing entry as "no minimap column".
        self.minimap = Vec::new();
        self.history = scope;
        self.list_state.select(Some(0));
        self.view_mode = ViewMode::Graph;
        self.search_results.clear();
        self.search_index = 0;
        self.status = None;
    }

    /// Restore the view parked when the first scope was opened — HEAD
    /// history normally, the all-branches history when the app started with
    /// `--all` — closing whichever scope (a file's history or all-branches)
    /// is currently open. No-op when nothing is parked.
    pub fn close_history(&mut self) -> bool {
        let Some(saved) = self.saved_history.take() else {
            return false;
        };
        let saved = *saved;
        self.commits = saved.commits;
        self.graph_rows = saved.graph_rows;
        self.minimap = saved.minimap;
        self.list_state.select(saved.selected);
        self.history = saved.scope;
        self.search_results.clear();
        self.search_index = 0;
        self.view_mode = ViewMode::Graph;
        self.status = None;
        true
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
            self.error("No repository loaded");
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
            Err(e) => self.error(format!("Failed to load branches & tags: {e}")),
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

/// One row of overlap is kept so the line you were reading is still on screen
/// after the jump, and the result is never zero: in a pane one or two rows tall
/// a page key must still move, or it looks broken.
fn page_size(height: u16) -> usize {
    (height as usize).saturating_sub(1).max(1)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_leaves_one_row_of_overlap() {
        assert_eq!(page_size(20), 19);
        assert_eq!(page_size(2), 1);
    }

    /// The bottom bar paints errors red, so a confirmation must not be one.
    #[test]
    fn a_notice_is_not_an_error() {
        assert!(Status::Error("boom".into()).is_error());
        assert!(!Status::Notice("3c4e053 sent to clipboard".into()).is_error());
        assert_eq!(Status::Notice("hello".into()).text(), "hello");
        assert_eq!(Status::Error("hello".into()).text(), "hello");
    }

    /// A pane can be one row tall, or zero before the first frame is drawn.
    /// Either way a page key has to move at least one row.
    #[test]
    fn a_page_is_never_zero_rows() {
        assert_eq!(page_size(1), 1);
        assert_eq!(page_size(0), 1);
    }

    #[test]
    fn ref_pane_rows_puts_every_ref_under_its_own_header() {
        let refs = RepoRefs {
            branches: vec![
                Branch::Local(RefName::new("main")),
                Branch::Remote(RefName::new("origin/main")),
            ],
            tags: vec![RefName::new("v0.1.0")],
        };

        let rows = ref_pane_rows(&refs);

        assert_eq!(
            rows,
            vec![
                RefPaneRow::Header("Local Branches"),
                RefPaneRow::Branch(Branch::Local(RefName::new("main"))),
                RefPaneRow::Header("Remote Branches"),
                RefPaneRow::Branch(Branch::Remote(RefName::new("origin/main"))),
                RefPaneRow::Header("Tags"),
                RefPaneRow::Tag(RefName::new("v0.1.0")),
            ]
        );
    }

    /// Empty sections keep their headers so the pane doesn't reshuffle as
    /// branches come and go.
    #[test]
    fn ref_pane_rows_keeps_headers_for_empty_sections() {
        let rows = ref_pane_rows(&RepoRefs {
            branches: Vec::new(),
            tags: Vec::new(),
        });
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(RefPaneRow::is_header));
    }

    #[test]
    fn stepping_skips_headers_in_both_directions() {
        let rows = ref_pane_rows(&RepoRefs {
            branches: vec![Branch::Local(RefName::new("main"))],
            tags: vec![RefName::new("v1")],
        });
        // rows: [Header, main, Header, Header, v1]
        let mut state = ListState::default();
        state.select(Some(1));

        step_selection_skipping(&mut state, &rows, 1, |r| r.is_header());
        assert_eq!(state.selected(), Some(4), "jumped over two headers");

        step_selection_skipping(&mut state, &rows, -1, |r| r.is_header());
        assert_eq!(state.selected(), Some(1), "and back again");
    }

    /// At the end of the list there is nothing selectable left, so the
    /// selection must stay put rather than landing on a header.
    #[test]
    fn stepping_past_the_last_entry_keeps_the_selection() {
        let rows = ref_pane_rows(&RepoRefs {
            branches: vec![Branch::Local(RefName::new("main"))],
            tags: Vec::new(),
        });
        let mut state = ListState::default();
        state.select(Some(1));

        step_selection_skipping(&mut state, &rows, 1, |r| r.is_header());
        assert_eq!(state.selected(), Some(1));
    }
}
