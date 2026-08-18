use gitloom::git::commit::CommitInfo;
use gitloom::graph::layout::GraphEngine;

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
fn test_linear_history() {
    let commits = vec![
        make_commit("3", &["2"]),
        make_commit("2", &["1"]),
        make_commit("1", &[]),
    ];

    let rows = GraphEngine::build(&commits);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].glyphs, "●");
    assert_eq!(rows[1].glyphs, "●");
    assert_eq!(rows[2].glyphs, "●");
}

#[test]
fn test_one_branch() {
    // 4
    // | \
    // 3  |
    // |  2
    // | /
    // 1
    let commits = vec![
        make_commit("4", &["3", "2"]),
        make_commit("3", &["1"]),
        make_commit("2", &["1"]),
        make_commit("1", &[]),
    ];

    let rows = GraphEngine::build(&commits);
    assert_eq!(rows[0].glyphs, "●");
    assert_eq!(rows[1].glyphs, "● │");
    assert_eq!(rows[2].glyphs, "│ ●");
    assert_eq!(rows[3].glyphs, "●");
}

#[test]
fn test_one_merge() {
    let commits = vec![
        make_commit("4", &["3", "2"]),
        make_commit("3", &["1"]),
        make_commit("2", &["1"]),
        make_commit("1", &[]),
    ];
    let rows = GraphEngine::build(&commits);
    assert_eq!(rows[0].glyphs, "●");
    assert_eq!(rows[1].glyphs, "● │");
    assert_eq!(rows[2].glyphs, "│ ●");
    assert_eq!(rows[3].glyphs, "●");
}

#[test]
fn test_multiple_branches() {
    let commits = vec![
        make_commit("5", &["4", "3"]),
        make_commit("4", &["0"]),
        make_commit("3", &["1", "2"]),
        make_commit("2", &["1"]),
        make_commit("1", &["0"]),
        make_commit("0", &[]),
    ];
    let rows = GraphEngine::build(&commits);
    assert_eq!(rows[0].glyphs, "●");
    assert_eq!(rows[1].glyphs, "● │");
    assert_eq!(rows[2].glyphs, "│ ●");
    assert_eq!(rows[3].glyphs, "│ │ ●");
    assert_eq!(rows[4].glyphs, "│ ●");
    assert_eq!(rows[5].glyphs, "●");
}

#[test]
fn test_branch_creation_and_termination() {
    let commits = vec![
        make_commit("4", &["3"]),
        make_commit("3", &["1", "2"]),
        make_commit("2", &[]),
        make_commit("1", &[]),
    ];
    let rows = GraphEngine::build(&commits);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].glyphs, "●");
    assert_eq!(rows[1].glyphs, "●");
    assert_eq!(rows[2].glyphs, "│ ●");
    assert_eq!(rows[3].glyphs, "●");
}

#[test]
fn test_complicated_merge_topology() {
    let commits = vec![
        make_commit("6", &["5", "4"]),
        make_commit("5", &["3"]),
        make_commit("4", &["3", "2"]),
        make_commit("3", &["1"]),
        make_commit("2", &["1"]),
        make_commit("1", &[]),
    ];
    let rows = GraphEngine::build(&commits);
    assert_eq!(rows.len(), 6);
}
