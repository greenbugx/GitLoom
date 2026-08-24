//! Background repository loading.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SendError, Sender};
use std::time::{Duration, Instant};

use crate::git::commit::CommitInfo;
use crate::git::repository::{self, GitRepository, Ref, RepoInfo};
use crate::graph::layout::{GraphEngine, GraphRow};

pub const COMMIT_LIMIT: usize = 1000;

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

pub struct RepoData {
    pub info: RepoInfo,
    pub commits: Vec<CommitInfo>,
    pub graph_rows: Vec<GraphRow>,
    pub ref_map: HashMap<String, Vec<Ref>>,
    pub minimap: Vec<char>,
}

/// A progress report from the loading thread
pub enum LoadMessage {
    Stage(LoadStage),
    Progress { done: usize, total: Option<usize> },
    Ready(Box<RepoData>),
    Failed(String),
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

pub fn spawn(path: PathBuf) -> Receiver<LoadMessage> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = load(path, &tx);
    });
    rx
}

fn load(path: PathBuf, tx: &Sender<LoadMessage>) -> Result<(), SendError<LoadMessage>> {
    tx.send(LoadMessage::Stage(LoadStage::Opening))?;

    let repo = match GitRepository::open(&path) {
        Ok(repo) => repo,
        Err(err) => return tx.send(LoadMessage::Failed(err.message().to_string())),
    };

    let info = repo.info();
    let git_dir = repo.path();

    tx.send(LoadMessage::Stage(LoadStage::History))?;
    let commits = repo
        .commits_with_progress(COMMIT_LIMIT, |done| {
            tx.send(LoadMessage::Progress { done, total: None }).is_ok()
        })
        .unwrap_or_default();

    tx.send(LoadMessage::Stage(LoadStage::Graph))?;
    let graph_rows = GraphEngine::build(&commits);

    tx.send(LoadMessage::Stage(LoadStage::Refs))?;
    let ref_map = repo.ref_map().unwrap_or_default();

    tx.send(LoadMessage::Stage(LoadStage::Minimap))?;
    let oids: Vec<String> = commits.iter().map(|c| c.oid.clone()).collect();
    let total = oids.len();
    let deltas = repository::commit_deltas_parallel(&git_dir, &oids, |done| {
        tx.send(LoadMessage::Progress {
            done,
            total: Some(total),
        })
        .is_ok()
    });

    tx.send(LoadMessage::Ready(Box::new(RepoData {
        info,
        commits,
        graph_rows,
        ref_map,
        minimap: minimap_chars(&deltas),
    })))
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
