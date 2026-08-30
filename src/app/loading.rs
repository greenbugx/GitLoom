//! Background repository loading, paginated.
//!
//! The HEAD history walk no longer stops at a fixed ceiling. It loads one
//! page behind the initial progress screen, then the thread stays alive —
//! same idea as [`crate::app::detail::DetailWorker`] — and answers
//! [`LoadRequest::NextPage`] by resuming the same `git2::Revwalk` where the
//! last page left off. A `Revwalk` cannot cross threads (`git2` types are not
//! `Send`), which is exactly why it has to live here rather than being
//! reconstructed by the app on demand.
//!
//! An `--all` startup load stays one-shot, capped at [`PAGE_SIZE`]: it is
//! opened far less often than HEAD history (once, only when the CLI flag is
//! given), and paginating it is future work rather than part of this pass.
//! An unborn HEAD — an empty repository, nothing committed yet — takes the
//! same one-shot path, since there is no commit for a paginated walk to
//! even start from.
//! Switching scopes *after* startup — `a`, `l`, `Esc` — goes through
//! [`crate::app::detail::DetailWorker`] instead of this module entirely and
//! is untouched by pagination either way.
//!
//! One rule, different from `DetailWorker`'s coalescing: page requests
//! accumulate, they are never coalesced. Coalescing keeps only the latest
//! of a backlog.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::git::commit::CommitInfo;
use crate::git::repository::{self, GitRepository, Ref, RepoInfo, WalkStart};
use crate::graph::layout::{GraphEngine, GraphRow};

/// Commits fetched per page, both for the initial HEAD-history load and every
/// subsequent [`LoadRequest::NextPage`]. Also the cap on the one-shot `--all`
/// startup load.
pub const PAGE_SIZE: usize = 1000;

/// Old name, kept as an alias: other one-shot loads elsewhere in the app
/// (`Request::FileHistory`, `Request::AllBranchesHistory`/`HeadHistory` in
/// [`crate::app::detail`]) read as "the usual cap on a single fetch", which is
/// still exactly what they do; only the HEAD-history walk in this module
/// became paginated.
pub const COMMIT_LIMIT: usize = PAGE_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStage {
    Opening,
    History,
    Graph,
    Refs,
    Minimap,
}

impl LoadStage {
    pub fn label(&self) -> &'static str {
        match self {
            LoadStage::Opening => "Opening repository",
            LoadStage::History => "Reading commit history",
            LoadStage::Graph => "Weaving commit graph",
            LoadStage::Refs => "Collecting branches & tags",
            LoadStage::Minimap => "Measuring commit sizes",
        }
    }
}

/// What only the first page brings: everything that describes the repository
/// itself rather than its history.
pub struct RepoInit {
    pub info: RepoInfo,
    pub ref_map: HashMap<String, Vec<Ref>>,
}

/// One page of history, laid out and measured, ready to append to
/// `AppState`'s vectors.
pub struct HistoryPage {
    pub commits: Vec<CommitInfo>,
    pub graph_rows: Vec<GraphRow>,
    pub minimap: Vec<char>,
    /// `true` when the revwalk had fewer than a full page left: there is
    /// nothing more to request. An empty page also sets this.
    pub is_last: bool,
}

/// A message from the loading thread.
pub enum LoadMessage {
    Stage(LoadStage),
    Progress {
        done: usize,
        total: Option<usize>,
    },
    /// The first page has landed, together with [`RepoInit`]. Sent exactly
    /// once per repository open.
    Ready {
        init: Box<RepoInit>,
        page: Box<HistoryPage>,
    },
    /// A page requested by [`LoadRequest::NextPage`] has landed. Never sent
    /// by a one-shot load (an `--all` startup load, or an empty repository),
    /// which has nothing listening for `LoadRequest` in the first place (see
    /// the module comment).
    PageReady(Box<HistoryPage>),
    Failed(String),
}

/// Work sent into the loading thread after the first page.
///
/// Presently just "give me the next page". A separate type from
/// [`LoadMessage`] rather than folded into it because the two travel in
/// opposite directions down two different channels — the same split as
/// [`crate::app::detail::Request`] and [`crate::app::detail::Payload`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadRequest {
    NextPage,
}

/// Everything the UI needs to draw the loading screen.
pub struct LoadingState {
    pub label: String,
    pub stage: LoadStage,
    pub done: usize,
    pub total: Option<usize>,
    pub started: Instant,
    /// Animation frame counter, advanced by `AppState::tick`.
    pub frame: usize,
}

impl LoadingState {
    pub fn new(label: String) -> Self {
        Self {
            label,
            stage: LoadStage::Opening,
            done: 0,
            total: None,
            started: Instant::now(),
            frame: 0,
        }
    }

    pub fn set_stage(&mut self, stage: LoadStage) {
        self.stage = stage;
        self.done = 0;
        self.total = None;
    }

    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    pub fn ratio(&self) -> Option<f64> {
        let total = self.total.filter(|t| *t > 0)?;
        Some((self.done as f64 / total as f64).clamp(0.0, 1.0))
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

/// Handle to the loading thread: sends page requests, receives messages.
///
/// Shaped like [`crate::app::detail::DetailWorker`] but without sequence
/// numbers or coalescing — see the module comment for why page requests
/// accumulate instead. `AppState` holds one of these for the lifetime of the
/// repository, the same way it holds a `DetailWorker`.
pub struct LoadWorker {
    tx: Sender<LoadRequest>,
    rx: Receiver<LoadMessage>,
}

impl LoadWorker {
    /// Start the background load behind the progress screen.
    ///
    /// `all_branches` mirrors the `--all` CLI flag: the history stage then
    /// walks every local branch tip, not just HEAD, so the graph's first
    /// frame is already the all-branches view. That walk stays one-shot,
    /// capped at [`PAGE_SIZE`].
    pub fn spawn(path: PathBuf, all_branches: bool) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<LoadRequest>();
        let (message_tx, message_rx) = mpsc::channel::<LoadMessage>();

        std::thread::spawn(move || worker(path, all_branches, request_rx, message_tx));

        Self {
            tx: request_tx,
            rx: message_rx,
        }
    }

    /// Ask for the next page. Returns `false` if the worker thread is gone
    /// (a one-shot load — an `--all` startup load, or an empty repository
    /// whose HEAD is unborn — which never reads this channel and exits after
    /// its one message, or a HEAD-history load that has already failed).
    pub fn request_next_page(&self) -> bool {
        self.tx.send(LoadRequest::NextPage).is_ok()
    }

    /// Drain every message currently waiting, passing each to `on_message` in
    /// order. Returns `false` once the channel has hung up, so the caller
    /// knows the thread is gone and can stop polling.
    pub fn poll(&self, mut on_message: impl FnMut(LoadMessage)) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(message) => on_message(message),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }
}

fn worker(path: PathBuf, all_branches: bool, rx: Receiver<LoadRequest>, tx: Sender<LoadMessage>) {
    if tx.send(LoadMessage::Stage(LoadStage::Opening)).is_err() {
        return;
    }

    let repo = match GitRepository::open(&path) {
        Ok(repo) => repo,
        Err(err) => {
            let _ = tx.send(LoadMessage::Failed(err.message().to_string()));
            return;
        }
    };
    let git_dir = repo.path();

    // An unborn HEAD — a fresh `git init` before the first commit — has an
    // empty history, not a broken one, so the repository must still open.
    // The paginated path below cannot do that: its first step seeds a walk
    // from HEAD and there is no commit to seed from. It therefore joins the
    // one-shot path, which already answers a failed walk with an empty,
    // final page plus the repository info — exactly what the startup loader
    // did before pagination existed.
    if !all_branches && !repo.head_is_unborn() {
        run_paginated_head_history(&repo, &git_dir, &rx, &tx);
        return;
    }

    let start = if all_branches {
        WalkStart::HeadAndLocalBranches
    } else {
        WalkStart::Head
    };
    // One-shot: build everything through the old capped path and exit.
    // Nothing will ever send this thread a `LoadRequest` in this mode.
    let _ = load_all_at_once(&repo, &git_dir, start, &tx);
}

/// Drives the paginated HEAD-history path: the first page synchronously,
/// then one more page per [`LoadRequest::NextPage`] until the walk is
/// exhausted, a channel breaks, or the walk itself errors (already reported
/// via `LoadMessage::Failed`).
///
/// Only called with a born HEAD: an unborn one (an empty repository) is
/// routed to [`load_all_at_once`] by [`worker`] instead, because the walk
/// this function seeds cannot start without a commit to push.
fn run_paginated_head_history(
    repo: &GitRepository,
    git_dir: &Path,
    rx: &Receiver<LoadRequest>,
    tx: &Sender<LoadMessage>,
) {
    let mut revwalk = match repo.walk_revwalk(WalkStart::Head) {
        Ok(revwalk) => revwalk,
        Err(err) => {
            tx.send(LoadMessage::Failed(err.message().to_string())).ok();
            return;
        }
    };

    if tx.send(LoadMessage::Stage(LoadStage::History)).is_err() {
        return;
    }
    let progress = |done: usize| tx.send(LoadMessage::Progress { done, total: None }).is_ok();
    let (commits, mut is_last) = match repo.next_page(&mut revwalk, PAGE_SIZE, progress) {
        Ok(page) => page,
        Err(err) => {
            tx.send(LoadMessage::Failed(err.message().to_string())).ok();
            return;
        }
    };

    let mut graph_engine = GraphEngine::new();
    let Some(first_page) = build_page(git_dir, &mut graph_engine, commits, is_last, tx) else {
        return;
    };

    let info = repo.info();
    tx.send(LoadMessage::Stage(LoadStage::Refs)).ok();
    let ref_map = repo.ref_map().unwrap_or_default();

    if tx
        .send(LoadMessage::Ready {
            init: Box::new(RepoInit { info, ref_map }),
            page: Box::new(first_page),
        })
        .is_err()
    {
        return;
    }

    // Idle until the app asks for more, or the walk is already exhausted.
    // `recv()` blocks this thread on purpose: there is nothing else for it
    // to do between requests.
    while !is_last {
        match rx.recv() {
            Ok(LoadRequest::NextPage) => {}
            Err(_) => return, // AppState (and its Sender) dropped; nothing left to serve
        }

        let (commits, last) = match repo.next_page(&mut revwalk, PAGE_SIZE, |_| true) {
            Ok(page) => page,
            Err(err) => {
                tx.send(LoadMessage::Failed(err.message().to_string())).ok();
                return;
            }
        };
        is_last = last;
        let Some(page) = build_page(git_dir, &mut graph_engine, commits, is_last, tx) else {
            return;
        };
        if tx.send(LoadMessage::PageReady(Box::new(page))).is_err() {
            return;
        }
    }
}

/// Turn a raw batch of commits into a [`HistoryPage`]: lay out its graph rows,
/// continuing `graph_engine`'s lane state from the previous page rather than
/// restarting it (so lanes don't reset at a page boundary), and measure its
/// minimap deltas. `None` means a channel send failed and the caller should
/// stop.
fn build_page(
    git_dir: &Path,
    graph_engine: &mut GraphEngine,
    commits: Vec<CommitInfo>,
    is_last: bool,
    tx: &Sender<LoadMessage>,
) -> Option<HistoryPage> {
    tx.send(LoadMessage::Stage(LoadStage::Graph)).ok()?;
    let graph_rows: Vec<GraphRow> = commits.iter().map(|c| graph_engine.process(c)).collect();

    tx.send(LoadMessage::Stage(LoadStage::Minimap)).ok()?;
    let oids: Vec<String> = commits.iter().map(|c| c.oid.clone()).collect();
    let total = oids.len();
    let deltas = repository::commit_deltas_parallel(git_dir, &oids, |done| {
        tx.send(LoadMessage::Progress {
            done,
            total: Some(total),
        })
        .is_ok()
    });

    Some(HistoryPage {
        commits,
        graph_rows,
        minimap: minimap_chars(&deltas),
        is_last,
    })
}

/// The old one-shot path: used for the `--all` startup load, and for any
/// repository whose HEAD is unborn (a fresh `git init`), whose empty
/// history arrives as one empty, final page. `None` means a channel send
/// failed.
fn load_all_at_once(
    repo: &GitRepository,
    git_dir: &Path,
    start: WalkStart,
    tx: &Sender<LoadMessage>,
) -> Option<()> {
    tx.send(LoadMessage::Stage(LoadStage::History)).ok()?;
    let progress = |done: usize| tx.send(LoadMessage::Progress { done, total: None }).is_ok();
    let commits = match start {
        WalkStart::Head => repo.commits_with_progress(COMMIT_LIMIT, progress),
        WalkStart::HeadAndLocalBranches => {
            repo.commits_all_branches_with_progress(COMMIT_LIMIT, progress)
        }
    }
    .unwrap_or_default();

    tx.send(LoadMessage::Stage(LoadStage::Graph)).ok()?;
    let graph_rows = GraphEngine::build(&commits);

    tx.send(LoadMessage::Stage(LoadStage::Refs)).ok()?;
    let ref_map = repo.ref_map().unwrap_or_default();
    let info = repo.info();

    tx.send(LoadMessage::Stage(LoadStage::Minimap)).ok()?;
    let oids: Vec<String> = commits.iter().map(|c| c.oid.clone()).collect();
    let total = oids.len();
    let deltas = repository::commit_deltas_parallel(git_dir, &oids, |done| {
        tx.send(LoadMessage::Progress {
            done,
            total: Some(total),
        })
        .is_ok()
    });

    tx.send(LoadMessage::Ready {
        init: Box::new(RepoInit { info, ref_map }),
        page: Box::new(HistoryPage {
            commits,
            graph_rows,
            minimap: minimap_chars(&deltas),
            is_last: true,
        }),
    })
    .ok()
}

pub fn minimap_chars(deltas: &[(usize, usize)]) -> Vec<char> {
    let max_delta = deltas.iter().map(|(i, d)| i + d).max().unwrap_or(0);
    deltas
        .iter()
        .map(|(i, d)| sparkline_char(i + d, max_delta))
        .collect()
}

const SPARKLINE_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn sparkline_char(value: usize, max: usize) -> char {
    if max == 0 {
        return SPARKLINE_CHARS[0];
    }
    let level = (value * 8).saturating_div(max).min(7);
    SPARKLINE_CHARS[level]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_scales_against_the_largest_commit() {
        assert_eq!(sparkline_char(0, 0), '▁');
        assert_eq!(sparkline_char(0, 100), '▁');
        assert_eq!(sparkline_char(50, 100), '▅');
        assert_eq!(sparkline_char(100, 100), '█');
    }

    #[test]
    fn minimap_has_one_char_per_commit() {
        let chars = minimap_chars(&[(1, 0), (0, 0), (5, 5)]);
        assert_eq!(chars.len(), 3);
        assert_eq!(chars[1], '▁');
        assert_eq!(chars[2], '█');
    }

    #[test]
    fn ratio_is_none_until_a_total_is_known() {
        let mut load = LoadingState::new("repo".to_string());
        assert_eq!(load.ratio(), None);

        load.done = 5;
        load.total = Some(0);
        assert_eq!(load.ratio(), None);

        load.total = Some(10);
        assert_eq!(load.ratio(), Some(0.5));
    }

    #[test]
    fn ratio_never_exceeds_one() {
        let mut load = LoadingState::new("repo".to_string());
        load.done = 20;
        load.total = Some(10);
        assert_eq!(load.ratio(), Some(1.0));
    }

    #[test]
    fn changing_stage_clears_the_previous_counters() {
        let mut load = LoadingState::new("repo".to_string());
        load.done = 900;
        load.total = Some(1000);

        load.set_stage(LoadStage::Minimap);

        assert_eq!(load.stage, LoadStage::Minimap);
        assert_eq!(load.done, 0);
        assert_eq!(load.ratio(), None);
    }

    #[test]
    fn tick_advances_the_animation_frame() {
        let mut load = LoadingState::new("repo".to_string());
        load.tick();
        load.tick();
        assert_eq!(load.frame, 2);
    }
}
