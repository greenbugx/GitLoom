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
