//! Off-thread detail fetches: commit details, changed files, diffs and
//! single-file history.
//!
//! Why this exists: the details/files/diff panes refresh on every `j`/`k`, and
//! each refresh re-reads the whole thing from libgit2. Done on the UI thread
//! that is a freeze proportional to the size of the commit, so holding `j`
//! through a large merge locks the terminal. Here the work happens on one
//! background thread and the UI keeps drawing.
//!
//! Two rules keep a burst of keypresses cheap and correct:
//!
//! - **Coalescing.** Requests that arrive while a fetch is running are drained
//!   and all but the last discarded. Scrolling past forty commits computes the
//!   one diff still on screen, not forty.
//! - **Sequencing.** Every request carries a number and the app only accepts a
//!   response matching its newest request, so a slow fetch that finishes after
//!   the user has moved on cannot repaint the pane with the wrong commit.
//!
//! Unlike [`crate::app::loading`], which loads a repository once behind a
//! progress screen, this worker lives for the whole session and answers many
//! small requests.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use crate::git::commit::{CommitDetails, CommitInfo};
use crate::git::repository::GitRepository;

/// Work for the detail thread. One variant per pane that reads from git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Details(String),
    Files(String),
    Diff(String),
    FileHistory { path: String, max_count: usize },
}

impl Request {
    pub fn pending_label(&self) -> &'static str {
        match self {
            Request::Details(_) => "Loading commit details...",
            Request::Files(_) => "Loading changed files...",
            Request::Diff(_) => "Loading diff...",
            Request::FileHistory { .. } => "Loading file history...",
        }
    }
}

/// A completed fetch, shaped to match the field it lands in.
#[derive(Debug, Clone)]
pub enum Payload {
    Details(Box<CommitDetails>),
    Files(Vec<String>),
    Diff(Vec<String>),
    FileHistory {
        path: String,
        commits: Vec<CommitInfo>,
    },
}

/// A reply from the worker, tagged with the sequence number of the request that
/// produced it so the app can drop replies it has outrun.
pub struct Response {
    pub seq: u64,
    pub result: Result<Payload, String>,
}

/// Handle to the detail thread: sends requests, receives replies, and tracks
/// which request is current.
pub struct DetailWorker {
    tx: Sender<(u64, Request)>,
    rx: Receiver<Response>,
    /// Sequence number of the most recent request; replies below it are stale.
    seq: u64,
    /// The in-flight request, or `None` when the worker is idle.
    pending: Option<Request>,
}

impl DetailWorker {
    /// Start a worker against the repository at `git_dir`.
    ///
    /// A `git2::Repository` is not `Send`, so the thread opens its own from the
    /// path rather than sharing the app's handle. This mirrors what
    /// [`crate::git::repository::commit_deltas_parallel`] does for the same
    /// reason.
    pub fn spawn(git_dir: PathBuf) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<(u64, Request)>();
        let (response_tx, response_rx) = mpsc::channel::<Response>();

        std::thread::spawn(move || worker(git_dir, request_rx, response_tx));

        Self {
            tx: request_tx,
            rx: response_rx,
            seq: 0,
            pending: None,
        }
    }

    /// Queue `request`, superseding whatever was in flight.
    ///
    /// Returns `false` if the worker thread is gone, which lets the caller
    /// report it instead of leaving a pane that never fills.
    pub fn request(&mut self, request: Request) -> bool {
        self.seq += 1;
        self.pending = Some(request.clone());
        if self.tx.send((self.seq, request)).is_err() {
            self.pending = None;
            return false;
        }
        true
    }

    /// The request being waited on, for the pane's placeholder text.
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }

    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Stop waiting for the in-flight request. The worker still computes it;
    /// this just means the answer will be ignored when it arrives, which is how
    /// closing a pane avoids being repainted a second later.
    pub fn cancel(&mut self) {
        self.pending = None;
        self.seq += 1;
    }

    /// Take the reply to the current request, if one has arrived.
    ///
    /// Replies to superseded requests are discarded here rather than by the
    /// caller, so `AppState` never sees a payload for a commit it has left.
    pub fn poll(&mut self) -> Option<Result<Payload, String>> {
        loop {
            match self.rx.try_recv() {
                Ok(response) if response.seq == self.seq => {
                    self.pending = None;
                    return Some(response.result);
                }
                // Stale: the answer to a commit the user has already scrolled
                // past. Keep draining, a fresher one may be right behind it.
                Ok(_) => continue,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    if self.pending.take().is_some() {
                        return Some(Err("detail worker stopped".to_string()));
                    }
                    return None;
                }
            }
        }
    }
}

fn worker(git_dir: PathBuf, rx: Receiver<(u64, Request)>, tx: Sender<Response>) {
    // Opened once and reused for every request. A failure is reported per
    // request rather than silently ending the thread, so a pane shows the error
    // instead of waiting forever.
    let repo = GitRepository::open(&git_dir);

    while let Ok(first) = rx.recv() {
        let (seq, request) = coalesce(first, &rx);
        let result = match &repo {
            Ok(repo) => run(repo, &request),
            Err(err) => Err(err.message().to_string()),
        };
        if tx.send(Response { seq, result }).is_err() {
            return;
        }
    }
}

/// Collapse a backlog to its last entry.
///
/// Held `j`/`k` enqueues a request per keypress while a slow fetch runs. Only
/// the last can still be the one on screen, so the rest are dropped unread
/// instead of each costing a full diff.
fn coalesce(first: (u64, Request), rx: &Receiver<(u64, Request)>) -> (u64, Request) {
    let mut latest = first;
    while let Ok(next) = rx.try_recv() {
        latest = next;
    }
    latest
}

fn run(repo: &GitRepository, request: &Request) -> Result<Payload, String> {
    match request {
        Request::Details(oid) => repo
            .commit_details(oid)
            .map(|details| Payload::Details(Box::new(details)))
            .map_err(|e| format!("Failed to load commit details: {}", e.message())),
        Request::Files(oid) => repo
            .changed_files(oid)
            .map(Payload::Files)
            .map_err(|e| format!("Failed to load changed files: {}", e.message())),
        Request::Diff(oid) => repo
            .commit_diff(oid)
            .map(Payload::Diff)
            .map_err(|e| format!("Failed to load diff: {}", e.message())),
        Request::FileHistory { path, max_count } => repo
            .file_history(path, *max_count)
            .map(|commits| Payload::FileHistory {
                path: path.clone(),
                commits,
            })
            .map_err(|e| format!("Failed to load history for {path}: {}", e.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalescing_keeps_only_the_last_of_a_backlog() {
        let (tx, rx) = mpsc::channel();
        for seq in 2..=5 {
            tx.send((seq, Request::Diff(format!("oid{seq}")))).unwrap();
        }

        let first = (1, Request::Diff("oid1".to_string()));
        let (seq, request) = coalesce(first, &rx);

        assert_eq!(seq, 5, "the newest request wins");
        assert_eq!(request, Request::Diff("oid5".to_string()));
    }

    #[test]
    fn coalescing_an_empty_queue_returns_the_only_request() {
        let (_tx, rx) = mpsc::channel::<(u64, Request)>();
        let first = (7, Request::Files("abc".to_string()));
        assert_eq!(coalesce(first.clone(), &rx), first);
    }

    /// A worker whose repository path is bogus should answer with an error
    /// rather than going quiet and leaving the pane on its placeholder.
    #[test]
    fn a_broken_repository_reports_instead_of_hanging() {
        let mut worker = DetailWorker::spawn(PathBuf::from("/definitely/not/a/repo"));
        assert!(worker.request(Request::Diff("deadbeef".to_string())));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(result) = worker.poll() {
                assert!(result.is_err(), "a missing repo cannot produce a diff");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "worker never replied");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!worker.is_busy(), "the reply clears the pending request");
    }

    #[test]
    fn cancelling_makes_the_in_flight_reply_stale() {
        let mut worker = DetailWorker::spawn(PathBuf::from("/definitely/not/a/repo"));
        worker.request(Request::Diff("deadbeef".to_string()));
        worker.cancel();

        assert!(!worker.is_busy());
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(worker.poll().is_none());
    }
}
