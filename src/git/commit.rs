/// A commit, snapshotted into owned data so it can outlive the `git2::Commit`
/// it was read from and cross a thread boundary (`git2` types are not `Send`).
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub summary: String,
    pub message: String,
}

impl CommitInfo {
    pub fn short_oid(&self) -> String {
        self.oid.chars().take(7).collect()
    }
}

#[derive(Debug, Clone)]
pub struct CommitDetails {
    pub oid: String,
    pub summary: String,
    pub message: String,
    pub author: String,
    pub date: i64,
    pub parents: Vec<String>,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}
