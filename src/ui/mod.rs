use crate::app::{AppState, RepoState};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, List, ListItem, Paragraph},
};

pub fn render(f: &mut Frame, state: &mut AppState) {
    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
    ]);
    let chunks = vertical.split(f.area());

    let title_block = Block::bordered().title("GitLoom");
    f.render_widget(title_block, chunks[0]);

    let horizontal = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]);
    let main_chunks = horizontal.split(chunks[1]);

    let graph_block = Block::bordered().title("GRAPH AREA");

    if state.commits.is_empty() {
        let content = match &state.repo_state {
            RepoState::None => "\n  No repository loaded".to_string(),
            RepoState::Error(err) => format!("\n  Error: {}", err),
            RepoState::Loaded(_, info) => {
                if info.is_bare {
                    format!("\n  Repository\n  {}\n\n  Bare repository", info.name)
                } else {
                    format!(
                        "\n  Repository\n  {}\n\n  Branch\n  {}\n\n  Status\n  {}",
                        info.name,
                        info.branch,
                        if info.is_clean { "Clean" } else { "Dirty" }
                    )
                }
            }
        };

        let graph_text = Paragraph::new(content).block(graph_block);
        f.render_widget(graph_text, main_chunks[0]);
    } else {
        let items: Vec<ListItem> = state
            .commits
            .iter()
            .zip(state.graph_rows.iter())
            .map(|(c, r)| {
                let content = format!("{}  {}  {}", r.glyphs, c.short_oid(), c.summary);
                ListItem::new(content)
            })
            .collect();

        let list = List::new(items)
            .block(graph_block)
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Yellow),
            )
            .highlight_symbol(">> ");

        f.render_stateful_widget(list, main_chunks[0], &mut state.list_state);
    }

    let details_block = Block::bordered().title(match state.view_mode {
        crate::app::ViewMode::Details => "DETAILS AREA",
        crate::app::ViewMode::Files => "CHANGED FILES",
        crate::app::ViewMode::Diff => "DIFF",
        crate::app::ViewMode::Graph => "DETAILS AREA",
    });

    match state.view_mode {
        crate::app::ViewMode::Details => {
            if let Some(details) = &state.commit_details {
                let date_str = chrono::DateTime::from_timestamp(details.date, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "Unknown".to_string());

                let parent_str = if details.parents.is_empty() {
                    "None".to_string()
                } else {
                    details.parents[0].clone()
                };

                let content = format!(
                    "\n  Commit\n  {}\n\n  {}\n\n  Author:\n  {}\n\n  Date: {}\n\n  Parent: {}\n\n  Files changed: {}\n\n  Insertions:\n  +{}\n\n  Deletions:\n  -{}",
                    details.oid.chars().take(7).collect::<String>(),
                    details.summary,
                    details.author,
                    date_str,
                    parent_str.chars().take(7).collect::<String>(),
                    details.files_changed,
                    details.insertions,
                    details.deletions
                );
                let details_text = Paragraph::new(content)
                    .block(details_block)
                    .scroll((state.details_scroll, state.details_scroll_x));
                f.render_widget(details_text, main_chunks[1]);
            } else {
                f.render_widget(details_block, main_chunks[1]);
            }
        }
        crate::app::ViewMode::Files => {
            let content = state.changed_files.join("\n");
            let details_text = Paragraph::new(content)
                .block(details_block)
                .scroll((state.details_scroll, state.details_scroll_x));
            f.render_widget(details_text, main_chunks[1]);
        }
        crate::app::ViewMode::Diff => {
            use ratatui::style::{Color, Style};
            use ratatui::text::{Line, Span};
            let mut lines = Vec::new();
            for line in &state.diff_lines {
                if line.starts_with('+') {
                    lines.push(Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(Color::Green),
                    )));
                } else if line.starts_with('-') {
                    lines.push(Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(Color::Red),
                    )));
                } else if line.starts_with("@@") {
                    lines.push(Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(Color::Cyan),
                    )));
                } else {
                    lines.push(Line::from(line.clone()));
                }
            }
            let details_text = Paragraph::new(lines)
                .block(details_block)
                .scroll((state.details_scroll, state.details_scroll_x));
            f.render_widget(details_text, main_chunks[1]);
        }
        crate::app::ViewMode::Graph => {
            f.render_widget(details_block, main_chunks[1]);
        }
    }

    let bottom_block = Block::bordered();
    let bottom_text = Paragraph::new(" ↑/↓ j/k Navigate/Scroll   Enter Details   f Files   d Diff   Esc Close   q Quit").block(bottom_block);
    f.render_widget(bottom_text, chunks[2]);
}
