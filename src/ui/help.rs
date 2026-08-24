//! The `?` keymap overlay.
//!
//! The keymap is data rather than a hand-written block of text, so the same
//! table can be rendered, measured for the overlay's width, and checked by tests.

use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};

/// A section of the keymap and its bindings.
pub struct Section {
    pub title: &'static str,
    pub bindings: &'static [(&'static str, &'static str)],
}

/// Every binding GitLoom answers to.
///
/// Kept in the same order as the key table in `main.rs` so the two can be read
/// side by side when either changes.
pub const SECTIONS: &[Section] = &[
    Section {
        title: "Navigate",
        bindings: &[
            (
                "j / k",
                "Down / up (moves the selection, or scrolls a pane)",
            ),
            (
                "J / K",
                "Next / previous commit, keeping the open pane in step",
            ),
            ("g / G", "Top / bottom of the focused pane"),
            ("Home / End", "Top / bottom, same as g / G"),
            ("PgUp / PgDn", "Move by a screenful"),
            ("h / l", "Scroll a wide pane left / right"),
        ],
    },
    Section {
        title: "Inspect",
        bindings: &[
            ("Enter", "Commit details"),
            ("f", "Changed files"),
            ("d", "Diff"),
            ("b", "Branches & tags"),
            ("l", "In the files pane: history of the selected file"),
            ("Esc", "Close the pane, or leave a file's history"),
        ],
    },
    Section {
        title: "Search",
        bindings: &[
            ("/", "Search summaries, authors and OIDs"),
            ("n / N", "Next / previous match"),
        ],
    },
    Section {
        title: "Other",
        bindings: &[
            ("y", "Copy the selected commit's full hash"),
            ("?", "Toggle this help"),
            ("q", "Quit"),
        ],
    },
];

/// Draw the overlay centered over `area`.
///
/// The area underneath is cleared first: this floats over the panes rather than
/// replacing a view mode, so dismissing it returns to whatever was open without
/// that pane having to remember anything.
pub fn render(f: &mut Frame, area: Rect) {
    let lines = help_lines();

    // Fit the overlay to its content, then clamp to the terminal so a small
    // window scrolls off the bottom rather than drawing outside the frame.
    let width = (content_width() as u16 + 4).min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);

    let [centered] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [centered] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(centered);

    let block = Block::bordered().title(" Keys ");
    f.render_widget(Clear, centered);
    f.render_widget(Paragraph::new(lines).block(block), centered);
}

/// The widest rendered line, used to size the overlay to its content.
fn content_width() -> usize {
    SECTIONS
        .iter()
        .flat_map(|section| section.bindings.iter())
        .map(|(keys, description)| {
            let key_column = KEY_COLUMN.max(keys.chars().count() + 1);
            key_column + description.chars().count()
        })
        .max()
        .unwrap_or(0)
}

/// Width of the key column, wide enough for the longest binding above.
const KEY_COLUMN: usize = 14;

fn help_lines() -> Vec<Line<'static>> {
    let key_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let title_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let mut lines = Vec::new();
    for (index, section) in SECTIONS.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(section.title, title_style)));
        for (keys, description) in section.bindings {
            let pad = KEY_COLUMN.saturating_sub(keys.chars().count()).max(1);
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(*keys, key_style),
                Span::raw(" ".repeat(pad)),
                Span::raw(*description),
            ]));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_documents_at_least_one_binding() {
        assert!(!SECTIONS.is_empty());
        for section in SECTIONS {
            assert!(
                !section.bindings.is_empty(),
                "section `{}` is empty",
                section.title
            );
            assert!(!section.title.is_empty());
        }
    }

    /// The overlay is the only place the keymap is published, so a binding
    /// missing here is invisible to users. These are the keys `main.rs`
    /// handles; add to both when adding a binding.
    #[test]
    fn the_documented_keys_cover_every_handled_key() {
        let documented: String = SECTIONS
            .iter()
            .flat_map(|s| s.bindings.iter())
            .map(|(keys, _)| *keys)
            .collect::<Vec<_>>()
            .join(" ");

        for key in [
            "j", "k", "J", "K", "g", "G", "PgUp", "PgDn", "Home", "End", "h", "l", "Enter", "f",
            "d", "b", "Esc", "/", "n", "N", "y", "?", "q",
        ] {
            assert!(documented.contains(key), "`{key}` is not documented");
        }
    }

    #[test]
    fn rendered_lines_cover_titles_and_bindings() {
        let bindings: usize = SECTIONS.iter().map(|s| s.bindings.len()).sum();
        // One line per binding, per title, plus a blank between sections.
        let expected = bindings + SECTIONS.len() + (SECTIONS.len() - 1);
        assert_eq!(help_lines().len(), expected);
    }

    #[test]
    fn the_overlay_is_wide_enough_for_its_widest_line() {
        let widest = help_lines()
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.chars().count())
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0);
        // Two leading spaces plus the border on each side.
        assert!(
            content_width() + 4 >= widest,
            "overlay {} narrower than its content {widest}",
            content_width() + 4
        );
    }
}
