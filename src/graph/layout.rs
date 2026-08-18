use crate::git::commit::CommitInfo;

#[derive(Debug, Clone, PartialEq)]
pub struct GraphRow {
    pub commit_oid: String,
    pub glyphs: String,
}

pub struct GraphEngine {
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

        let mut glyphs = String::new();
        for i in 0..self.lanes.len() {
            if i == col {
                glyphs.push('●');
            } else if self.lanes[i].is_some() {
                glyphs.push('│');
            } else {
                glyphs.push(' ');
            }
            if i < self.lanes.len() - 1 {
                glyphs.push(' ');
            }
        }

        if commit.parents.is_empty() {
            self.lanes[col] = None;
        } else {
            // First parent: only take current col if it's not already expected somewhere else
            if self
                .lanes
                .iter()
                .any(|oid| oid.as_deref() == Some(&commit.parents[0]))
            {
                self.lanes[col] = None;
            } else {
                self.lanes[col] = Some(commit.parents[0].clone());
            }
            // Other parents
            for parent in commit.parents.iter().skip(1) {
                if !self.lanes.iter().any(|oid| oid.as_deref() == Some(parent)) {
                    if let Some(empty_idx) = self.lanes.iter().position(|x| x.is_none()) {
                        self.lanes[empty_idx] = Some(parent.clone());
                    } else {
                        self.lanes.push(Some(parent.clone()));
                    }
                }
            }
        }

        while self.lanes.last() == Some(&None) {
            self.lanes.pop();
        }

        GraphRow {
            commit_oid: commit.oid.clone(),
            glyphs,
        }
    }

    pub fn build(commits: &[CommitInfo]) -> Vec<GraphRow> {
        let mut engine = Self::new();
        commits.iter().map(|c| engine.process(c)).collect()
    }
}
