mod loading;

use crate::app::{AppState, RepoState};
use crate::git::repository::RefKind;
use crate::graph::layout::lane_color;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
};

/// The highlight symbol shown on the selected graph row. Its width is reserved
/// by the `List` for every row (blank for unselected rows), so it must stay in
/// sync with `HIGHLIGHT_WIDTH` used to right-align the minimap.
const HIGHLIGHT_SYMBOL: &str = ">> ";
const HIGHLIGHT_WIDTH: usize = 3;

pub fn render(f: &mut Frame, state: &mut AppState) {
    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
    ]);
    let chunks = vertical.split(f.area());

    let title_block = Block::bordered().title("GitLoom");
    f.render_widget(title_block, chunks[0]);

    let horizontal = Layout::horizontal([Constraint::Percentage(64), Constraint::Percentage(36)]);
    let main_chunks = horizontal.split(chunks[1]);

    let graph_block = Block::bordered().title("GRAPH AREA");

    if state.commits.is_empty() {
        if let Some(load) = &state.loading {
            loading::render(f, main_chunks[0], graph_block, load);
        } else {
            let content = match &state.repo_state {
                RepoState::None => "\n  No repository loaded".to_string(),
                RepoState::Error(err) => format!("\n  Error: {}", err),
                RepoState::Loading(_) => "\n  Opening repository...".to_string(),
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
        }
    } else {
        // Cap how much horizontal room the graph glyphs may take so a wide
        // graph cannot crowd out the commit summaries. Lanes beyond this are
        // not rendered in the UI (the full topology is still in the engine).
        const MAX_GRAPH_LANES: usize = 24;
        let inner_width =
            (main_chunks[0].width.saturating_sub(2) as usize).saturating_sub(HIGHLIGHT_WIDTH);
        let items: Vec<ListItem> = state
            .commits
            .iter()
            .zip(state.graph_rows.iter())
            .enumerate()
            .map(|(i, (c, r))| {
                let mut spans: Vec<Span> = Vec::new();
                for seg in r.segments.iter().take(MAX_GRAPH_LANES) {
                    let ch = seg.glyph.char();
                    if ch == ' ' {
                        spans.push(Span::raw(" "));
                    } else {
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(lane_color(seg.lane)),
                        ));
                    }
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    c.short_oid(),
                    Style::default().fg(Color::DarkGray),
                ));

                // Inline ref badges next to the commit summary: branches green,
                // remote branches red, tags yellow.
                if let Some(badges) = state.ref_map.get(&c.oid) {
                    for badge in badges {
                        let color = match badge.kind {
                            RefKind::Local => Color::Green,
                            RefKind::Remote => Color::Red,
                            RefKind::Tag => Color::Yellow,
                        };
                        spans.push(Span::raw(" "));
                        spans.push(Span::styled(
                            badge.name.clone(),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ));
                    }
                }

                spans.push(Span::raw("  "));
                spans.extend(style_summary(&c.summary));

                let mut line = Line::from(spans);
                if let Some(&ch) = state.minimap.get(i) {
                    let content_width = line.width();
                    // Reserve a right-aligned minimap column only when the row
                    // fits; if it would push the commit summary past the edge,
                    // yield to the summary instead (no extra truncation).
                    let pad = inner_width.saturating_sub(content_width + 1);
                    if pad > 0 {
                        line.spans.push(Span::raw(" ".repeat(pad)));
                        line.spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .block(graph_block)
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Yellow),
            )
            .highlight_symbol(HIGHLIGHT_SYMBOL);

        f.render_stateful_widget(list, main_chunks[0], &mut state.list_state);
    }

    let details_block = Block::bordered().title(match state.view_mode {
        crate::app::ViewMode::Details => "DETAILS AREA",
        crate::app::ViewMode::Files => "CHANGED FILES",
        crate::app::ViewMode::Diff => "DIFF",
        crate::app::ViewMode::Graph => "DETAILS AREA",
        crate::app::ViewMode::Refs => "BRANCHES & TAGS",
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
        crate::app::ViewMode::Refs => {
            if let Some(refs) = &state.refs {
                let mut items = Vec::new();

                use ratatui::style::{Color, Modifier, Style};
                let header_style = Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD);
                let item_style = Style::default().fg(Color::White);

                items.push(ListItem::new("Local Branches").style(header_style));
                for branch in &refs.local_branches {
                    items.push(ListItem::new(format!("  {}", branch)).style(item_style));
                }

                items.push(ListItem::new("Remote Branches").style(header_style));
                for branch in &refs.remote_branches {
                    items.push(ListItem::new(format!("  {}", branch)).style(item_style));
                }

                items.push(ListItem::new("Tags").style(header_style));
                for tag in &refs.tags {
                    items.push(ListItem::new(format!("  {}", tag)).style(item_style));
                }

                let list = List::new(items)
                    .block(details_block)
                    .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White));

                // clone state.refs_list_state since render_stateful_widget takes a mut ref
                let mut list_state = state.refs_list_state;
                f.render_stateful_widget(list, main_chunks[1], &mut list_state);
            } else {
                f.render_widget(details_block, main_chunks[1]);
            }
        }
    }

    let bottom_block = Block::bordered();

    let bottom_text = if state.is_searching {
        Paragraph::new(format!("/{}", state.search_query)).block(bottom_block)
    } else if let Some(load) = &state.loading {
        Paragraph::new(loading::status_line(load)).block(bottom_block)
    } else if !state.search_results.is_empty() {
        let text = format!(
            " ↑/↓ j/k Nav   Enter Details   f Files   d Diff   b Branches   / Search   n/N Match {}/{}   Esc Close   q Quit",
            state.search_index + 1,
            state.search_results.len()
        );
        Paragraph::new(text).block(bottom_block)
    } else {
        Paragraph::new(" ↑/↓ j/k Navigate/Scroll   Enter Details   f Files   d Diff   b Branches   / Search   Esc Close   q Quit").block(bottom_block)
    };

    f.render_widget(bottom_text, chunks[2]);
}

/// Color the conventional-commit prefix of a summary, leaving the rest plain:
/// feat green, fix red, docs blue, chore gray, refactor magenta, test cyan.
fn style_summary(summary: &str) -> Vec<Span<'static>> {
    const PREFIXES: [(&str, Color); 6] = [
        ("feat:", Color::Green),
        ("fix:", Color::Red),
        ("docs:", Color::Blue),
        ("chore:", Color::DarkGray),
        ("refactor:", Color::Magenta),
        ("test:", Color::Cyan),
    ];
    for (prefix, color) in PREFIXES {
        if summary.starts_with(prefix) {
            let (prefix_part, rest) = summary.split_at(prefix.len());
            return vec![
                Span::styled(
                    prefix_part.to_string(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(rest.to_string()),
            ];
        }
    }
    vec![Span::raw(summary.to_string())]
}
