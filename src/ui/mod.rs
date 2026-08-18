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
            .map(|c| {
                let content = format!("{} {}", c.short_oid(), c.summary);
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

    let details_block = Block::bordered().title("DETAILS AREA");
    f.render_widget(details_block, main_chunks[1]);

    let bottom_block = Block::bordered();
    let bottom_text = Paragraph::new(" ↑/↓ j/k Navigate   q Quit").block(bottom_block);
    f.render_widget(bottom_text, chunks[2]);
}
