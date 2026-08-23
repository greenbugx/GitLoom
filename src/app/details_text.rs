use crate::git::commit::CommitDetails;

/// Builds the exact text shown in the Details pane. Used by both `ui::render`
/// and `AppState::content_dimensions` (to clamp scrolling
/// against it, by measuring this same string), so the two can never disagree
/// about how many lines or how wide the content is.
pub fn format(details: &CommitDetails) -> String {
    let date_str = chrono::DateTime::from_timestamp(details.date, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let parent_str = if details.parents.is_empty() {
        "None".to_string()
    } else {
        details.parents[0].chars().take(7).collect()
    };

    format!(
        "\n  Commit\n  {}\n\n  {}\n\n  Author:\n  {}\n\n  Date: {}\n\n  Parent: {}\n\n  Files changed: {}\n\n  Insertions:\n  +{}\n\n  Deletions:\n  -{}",
        details.oid.chars().take(7).collect::<String>(),
        details.summary,
        details.author,
        date_str,
        parent_str,
        details.files_changed,
        details.insertions,
        details.deletions
    )
}
