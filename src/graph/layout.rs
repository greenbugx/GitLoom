//! Structured graph layout engine.
//!
//! ## Data model constraint: strict 1-row-per-commit
//!
//! This layout deliberately keeps one `GraphRow` per `CommitInfo`, there are
//! NO spacer rows between commits. Tools like `tig` or `git log --graph` insert
//! dedicated spacer rows (e.g. `|\` and `|/`) to draw diagonal connectors
//! between two consecutive commit rows. We compress those connectors into the
//! same row as the commit that owns them:
//!
//! * An edge that leaves a node and reaches a lane to the RIGHT is drawn as a
//!   `DiagDown` (`╲`) glyph in that lane of the node's own row.
//! * An edge that leaves a node and reaches a lane to the LEFT is drawn as a
//!   `DiagUp` (`╱`) glyph in that lane of the node's own row.
//! * When an edge joins an existing (through) line in another lane, a corner
//!   glyph (`MergeLeft`/`MergeRight`) is drawn instead, so the join stays within
//!   one row. This means two crossings can compress into a single row; that is
//!   an intentional, documented tradeoff, not an oversight.
//!
//! The consequence: `AppState.list_state` continues to map 1:1 onto
//! `commits[i]`, so selection/scroll indexing needs no adjustment.
//!
//! ## Color keying
//!
//! `lane_color` keys colors by lane SLOT, not by branch identity. When a lane
//! is freed and later reused by a different branch, its color is reused too.
//! This is a deliberate tradeoff: tracking stable per-branch colors would
//! require the engine to know future commit topology, which is impossible in a
//! single forward pass. The reuse is expected behavior, not a bug.

use crate::git::commit::CommitInfo;
use ratatui::style::Color;

/// The glyph drawn at a single lane of a graph row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphType {
    /// The commit node itself.
    Node,
    /// A line that passes straight through this row (no node here).
    Vertical,
    /// A line joined by an edge from a node to its RIGHT (edge descends toward
    /// the left and merges into the line; `┌`). Drawn at a through-lane whose
    /// pending commit is a parent of this row's node.
    MergeLeft,
    /// A line joined by an edge from a node to its LEFT (edge descends toward
    /// the right and merges into the line; `┐`). Drawn at a through-lane whose
    /// pending commit is a parent of this row's node.
    MergeRight,
    /// A through-line turning toward the left at the bottom (`└`). Reserved:
    /// the current lane-occupancy state machine represents divergence via
    /// `DiagDown`/`DiagUp` instead, so this is never emitted today.
    BranchLeft,
    /// A through-line turning toward the right at the bottom (`┘`). Reserved,
    /// for the same reason as `BranchLeft`.
    BranchRight,
    /// A fresh edge descending to the right (`╲`), from a node to a new lane.
    DiagDown,
    /// A fresh edge descending to the left (`╱`), from a node to a new lane.
    DiagUp,
    /// A horizontal connector (`─`) filling empty lanes between a node and the
    /// lane its edge reaches.
    HorizDash,
    /// An unused lane.
    Empty,
}

impl GlyphType {
    pub fn char(&self) -> char {
        match self {
            GlyphType::Node => '●',
            GlyphType::Vertical => '│',
            GlyphType::MergeLeft => '╭',
            GlyphType::MergeRight => '╮',
            GlyphType::BranchLeft => '╰',
            GlyphType::BranchRight => '╯',
            GlyphType::DiagDown => '╲',
            GlyphType::DiagUp => '╱',
            GlyphType::HorizDash => '─',
            GlyphType::Empty => ' ',
        }
    }
}

/// One glyph at one lane of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphSegment {
    pub lane: usize,
    pub glyph: GlyphType,
}

/// The structured graph row for a single commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub commit_oid: String,
    /// One segment per lane, covering lanes `0..=max_lane` inclusive
    /// missing lanes are represented with `GlyphType::Empty`
    pub segments: Vec<GraphSegment>,
    /// The lane the commit node is drawn in
    pub node_lane: usize,
}

impl GraphRow {
    /// Derive the legacy flat-glyph representation used by the original engine
    /// and by `tests/graph_layout.rs`.
    ///
    /// Mapping back to the flat form:
    /// * `Node` -> `●`
    /// * `Vertical`, `MergeLeft`, `MergeRight`, `BranchLeft`, `BranchRight` ->
    ///   `│` (they all occupy an active lane)
    /// * every other glyph -> ` ` (empty space)
    ///
    /// Lanes are rendered with a single space between them and trailing empty
    /// lanes are trimmed, reproducing the old output exactly.
    pub fn render_plain(&self) -> String {
        let max_lane = self.segments.iter().map(|s| s.lane).max().unwrap_or(0);
        let mut chars = vec![' '; max_lane * 2 + 1];
        for seg in &self.segments {
            let ch = match seg.glyph {
                GlyphType::Node => '●',
                GlyphType::Vertical
                | GlyphType::MergeLeft
                | GlyphType::MergeRight
                | GlyphType::BranchLeft
                | GlyphType::BranchRight => '│',
                _ => ' ',
            };
            chars[seg.lane * 2] = ch;
        }
        let s: String = chars.into_iter().collect();
        s.trim_end().to_string()
    }
}

/// Cycle through 8 ANSI colors, keyed by lane slot
///
/// See the module-level comment for why this is slot-keyed rather than branch-identity-keyed.
pub fn lane_color(lane: usize) -> Color {
    const COLORS: [Color; 8] = [
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::White,
        Color::Black,
    ];
    COLORS[lane % COLORS.len()]
}

pub struct GraphEngine {
    /// Active lanes. Each holds the oid of the commit that is expected to
    /// arrive in that lane next.
    lanes: Vec<Option<String>>,
}

impl Default for GraphEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphEngine {
    pub fn new() -> Self {
        Self { lanes: Vec::new() }
    }

    pub fn process(&mut self, commit: &CommitInfo) -> GraphRow {
        let col = match self
            .lanes
            .iter()
            .position(|oid| oid.as_deref() == Some(&commit.oid))
        {
            Some(idx) => idx,
            None => {
                if let Some(empty_idx) = self.lanes.iter().position(|x| x.is_none()) {
                    self.lanes[empty_idx] = Some(commit.oid.clone());
                    empty_idx
                } else {
                    self.lanes.push(Some(commit.oid.clone()));
                    self.lanes.len() - 1
                }
            }
        };

        // Simulate the lane transition exactly as the legacy engine did, so the
        // derived flat rendering (`render_plain`) is byte-for-byte identical to
        // the old output. `targets` records, for every parent of `commit`,
        // the lane its edge reaches in the next state.
        let (after, targets) = Self::next_state(&self.lanes, col, commit);

        let mut row_glyphs: Vec<Option<GlyphType>> = vec![None; after.len()];

        // The node.
        row_glyphs[col] = Some(GlyphType::Node);

        // Through lanes: any active lane that is not the node lane keeps its
        //    line running straight down this row.
        for (j, slot) in self.lanes.iter().enumerate() {
            if j != col && slot.is_some() {
                row_glyphs[j] = Some(GlyphType::Vertical);
            }
        }

        // Exit edges from the node to each parent's target lane.
        for &t in &targets {
            if t == col {
                continue;
            }
            let through = self.lanes.get(t).is_some_and(|s| s.is_some());
            let glyph = if through {
                // The edge joins an existing line: use a corner so the join
                // stays in this single row.
                if t > col {
                    GlyphType::MergeRight
                } else {
                    GlyphType::MergeLeft
                }
            } else if t > col {
                GlyphType::DiagDown
            } else {
                GlyphType::DiagUp
            };
            row_glyphs[t] = Some(glyph);
        }

        // Horizontal connectors across empty lanes between the node and the
        //    lane its edge reaches (only for lanes not otherwise occupied).
        for &t in &targets {
            if t == col {
                continue;
            }
            let (lo, hi) = if t > col { (col + 1, t) } else { (t + 1, col) };
            for slot in &mut row_glyphs[lo..hi] {
                if slot.is_none() {
                    *slot = Some(GlyphType::HorizDash);
                }
            }
        }

        // Emit segments, trimming trailing empty lanes.
        let mut segments = Vec::new();
        for (j, g) in row_glyphs.into_iter().enumerate() {
            segments.push(GraphSegment {
                lane: j,
                glyph: g.unwrap_or(GlyphType::Empty),
            });
        }
        while segments.last().is_some_and(|s| s.glyph == GlyphType::Empty) {
            segments.pop();
        }

        self.lanes = after;
        while self.lanes.last() == Some(&None) {
            self.lanes.pop();
        }

        GraphRow {
            commit_oid: commit.oid.clone(),
            segments,
            node_lane: col,
        }
    }

    /// Compute the post-commit lane state and the target lane of every parent edge.
    /// Mirrors the legacy update rules exactly:
    ///
    /// * root commit frees the node's lane;
    /// * the first parent keeps the node's lane unless it is already expected
    ///   in another lane, in which case the node's lane is freed;
    /// * each remaining parent is placed in the first free lane (or appended).
    fn next_state(
        lanes: &[Option<String>],
        col: usize,
        commit: &CommitInfo,
    ) -> (Vec<Option<String>>, Vec<usize>) {
        let mut after = lanes.to_vec();
        let mut targets = Vec::new();

        if commit.parents.is_empty() {
            after[col] = None;
        } else {
            if let Some(m) = after
                .iter()
                .position(|o| o.as_deref() == Some(&commit.parents[0]))
            {
                after[col] = None;
                if m != col {
                    targets.push(m);
                }
            } else {
                after[col] = Some(commit.parents[0].clone());
            }
            for parent in commit.parents.iter().skip(1) {
                if let Some(m) = after.iter().position(|o| o.as_deref() == Some(parent)) {
                    if m != col {
                        targets.push(m);
                    }
                } else {
                    let t = match after.iter().position(|x| x.is_none()) {
                        Some(idx) => idx,
                        None => after.len(),
                    };
                    if t == after.len() {
                        after.push(Some(parent.clone()));
                    } else {
                        after[t] = Some(parent.clone());
                    }
                    if t != col {
                        targets.push(t);
                    }
                }
            }
        }

        (after, targets)
    }

    pub fn build(commits: &[CommitInfo]) -> Vec<GraphRow> {
        let mut engine = Self::new();
        commits.iter().map(|c| engine.process(c)).collect()
    }

    /// One node per commit in a single lane, for *filtered* histories.
    ///
    /// [`GraphEngine::build`] must not be used on a filtered list. A lane is
    /// held open until the commit it is waiting for arrives, so when a commit's
    /// real parents have been filtered out, their lanes are never claimed: each
    /// commit takes a fresh lane and the graph widens into a staircase one lane
    /// per row. A single-path history filters out nearly every parent, so that
    /// is the normal case there, not an edge case.
    ///
    /// Drawing the filtered list as a straight line is also the honest
    /// rendering: consecutive rows are ancestors, but usually not parent and
    /// child, so no branch or merge topology between them can be shown. This is
    /// the same simplification `git log -- <path>` makes without `--graph`.
    pub fn build_linear(commits: &[CommitInfo]) -> Vec<GraphRow> {
        commits
            .iter()
            .map(|c| GraphRow {
                commit_oid: c.oid.clone(),
                segments: vec![GraphSegment {
                    lane: 0,
                    glyph: GlyphType::Node,
                }],
                node_lane: 0,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::commit::CommitInfo;

    fn make_commit(oid: &str, parents: &[&str]) -> CommitInfo {
        CommitInfo {
            oid: oid.to_string(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            author: "Test".to_string(),
            timestamp: 0,
            summary: "test".to_string(),
            message: "test".to_string(),
        }
    }

    #[test]
    fn row_count_tracks_commit_count() {
        let commits = vec![
            make_commit("3", &["2"]),
            make_commit("2", &["1"]),
            make_commit("1", &[]),
        ];
        let rows = GraphEngine::build(&commits);
        assert_eq!(rows.len(), commits.len());
        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(rows[1].node_lane, 0);
        assert_eq!(rows[2].node_lane, 0);
    }

    #[test]
    fn render_plain_matches_legacy_output() {
        let commits = vec![
            make_commit("4", &["3", "2"]),
            make_commit("3", &["1"]),
            make_commit("2", &["1"]),
            make_commit("1", &[]),
        ];
        let rows = GraphEngine::build(&commits);
        assert_eq!(rows[0].render_plain(), "●");
        assert_eq!(rows[1].render_plain(), "● │");
        assert_eq!(rows[2].render_plain(), "│ ●");
        assert_eq!(rows[3].render_plain(), "●");
    }

    #[test]
    fn structured_segments_carry_diagonals() {
        let commits = vec![
            make_commit("4", &["3", "2"]),
            make_commit("3", &["1"]),
            make_commit("2", &["1"]),
            make_commit("1", &[]),
        ];
        let rows = GraphEngine::build(&commits);
        // row 0: node at lane 0, fresh branch edge to lane 1 (down-right).
        assert_eq!(
            rows[0].segments,
            vec![
                GraphSegment {
                    lane: 0,
                    glyph: GlyphType::Node
                },
                GraphSegment {
                    lane: 1,
                    glyph: GlyphType::DiagDown
                },
            ]
        );
        // row 2: node 2 at lane 1 merges into the through-line at lane 0.
        assert_eq!(
            rows[2].segments,
            vec![
                GraphSegment {
                    lane: 0,
                    glyph: GlyphType::MergeLeft
                },
                GraphSegment {
                    lane: 1,
                    glyph: GlyphType::Node
                },
            ]
        );
    }

    /// A file's history is a subset of the walk, so consecutive rows are
    /// usually not parent and child. Every row stays in lane 0 rather than
    /// claiming a new one.
    #[test]
    fn a_filtered_history_stays_in_one_lane() {
        let commits = vec![
            make_commit("9", &["8"]),
            make_commit("5", &["4"]),
            make_commit("1", &[]),
        ];
        let rows = GraphEngine::build_linear(&commits);

        assert_eq!(rows.len(), 3);
        for (row, commit) in rows.iter().zip(commits.iter()) {
            assert_eq!(row.commit_oid, commit.oid);
            assert_eq!(row.node_lane, 0);
            assert_eq!(row.render_plain(), "●");
        }
    }

    /// The failure `build_linear` exists to avoid: fed the same gapped list,
    /// the graph engine widens by a lane per row because the parents it is
    /// waiting for never arrive.
    #[test]
    fn the_graph_engine_would_widen_on_the_same_list() {
        let commits = vec![
            make_commit("9", &["8"]),
            make_commit("5", &["4"]),
            make_commit("1", &[]),
        ];
        let rows = GraphEngine::build(&commits);
        assert!(
            rows[2].node_lane > 0,
            "expected a staircase, got lane {}",
            rows[2].node_lane
        );
    }

    #[test]
    fn an_empty_filtered_history_has_no_rows() {
        assert!(GraphEngine::build_linear(&[]).is_empty());
    }
}
