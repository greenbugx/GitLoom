//! The loading screen and its status-bar counterpart.
//!
//! The graph pane shows a loom weaving the commit lanes. Every frame is derived
//! from `LoadingState::frame` alone, so the layout maths is unit testable.

use crate::app::loading::LoadingState;
use crate::graph::layout::lane_color;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use std::time::Duration;

/// Six keeps every thread on a visible
/// `lane_color`, avoiding the white and black slots of the palette.
const WEAVE_LANES: usize = 6;
/// Weft rows woven before the cloth is finished and the pattern restarts.
const WEAVE_ROWS: usize = 4;
const BAR_WIDTH: usize = 24;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn render(f: &mut Frame, area: Rect, block: Block<'_>, load: &LoadingState) {
    // The block borders eat one cell on each side.
    let width = area.width.saturating_sub(2) as usize;
    let height = area.height.saturating_sub(2) as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(center(width, vec![dim(load.label.clone())]));
    lines.push(Line::from(""));

    for row in weave_rows(load.frame, WEAVE_LANES, WEAVE_ROWS) {
        lines.push(center(width, weave_spans(&row)));
    }

    lines.push(Line::from(""));
    lines.push(center(width, vec![bright(load.stage.label())]));
    lines.push(Line::from(""));
    lines.push(center(width, bar_spans(load)));
    lines.push(Line::from(""));
    lines.push(center(width, vec![dim(counters(load))]));
    lines.push(Line::from(""));
    lines.push(center(width, vec![key("q"), dim("  cancel")]));

    let padding = height.saturating_sub(lines.len()) / 2;
    let mut content = vec![Line::from(""); padding];
    content.extend(lines);

    f.render_widget(Paragraph::new(content).block(block), area);
}

/// One-line echo of the loading screen for the status bar.
pub fn status_line(load: &LoadingState) -> Line<'static> {
    let spinner = SPINNER[load.frame % SPINNER.len()].to_string();
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(spinner, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(load.stage.label(), Style::default().fg(Color::White)),
    ];
    if let Some(ratio) = load.ratio() {
        spans.push(dim(format!("  {:>3}%", percent(ratio))));
    }
    spans.push(dim(format!("   ·   {}", format_elapsed(load.elapsed()))));
    Line::from(spans)
}

/// The status-bar line shown while a background page loads — pagination's
/// counterpart to `status_line`, used once the graph already has commits on
/// screen and the full-screen loader (and its own `LoadingState`) no longer
/// applies. Takes the raw frame counter directly rather than a
/// `LoadingState`, since `AppState.loading` is `None` throughout background
/// pagination.
///
/// `searching_for`, when set, means this page was requested by a search that
/// came up empty against what's loaded so far (see `AppState::execute_search`
/// and `retry_search_if_pending`): the message says so, rather than a bare
/// "loading more" that would leave the search silently stalled from the
/// user's point of view while pages load looking for their query.
pub fn loading_more_line(frame: usize, searching_for: Option<&str>) -> Line<'static> {
    let spinner = SPINNER[frame % SPINNER.len()].to_string();
    let label = match searching_for {
        Some(query) => format!("Searching \"{query}\"… loading more history"),
        None => "Loading more history…".to_string(),
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(spinner, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(label, Style::default().fg(Color::White)),
    ])
}

fn bar_spans(load: &LoadingState) -> Vec<Span<'static>> {
    let bar = progress_bar(BAR_WIDTH, load.ratio(), load.frame);
    let mut spans = vec![Span::styled(bar, Style::default().fg(Color::Cyan))];
    if let Some(ratio) = load.ratio() {
        spans.push(dim(format!("  {:>3}%", percent(ratio))));
    }
    spans
}

fn dim(text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(Color::DarkGray))
}

fn bright(text: impl Into<String>) -> Span<'static> {
    let style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    Span::styled(text.into(), style)
}

fn key(name: &'static str) -> Span<'static> {
    let style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    Span::styled(name, style)
}

/// The weave grid at `frame`, one string per row.
///
/// Warp threads sit at even character positions, weft fills the odd gaps. Rows
/// above the shuttle are finished cloth (`┼─┼─┼`), its own row is woven up to
/// where it has reached, rows below are bare warp (`│ │ │`). The shuttle
/// reverses direction each row and the pattern restarts after the last one.
pub fn weave_rows(frame: usize, lanes: usize, rows: usize) -> Vec<String> {
    if lanes == 0 || rows == 0 {
        return Vec::new();
    }

    let position = frame % (rows * lanes);
    let shuttle_row = position / lanes;
    // Lanes the shuttle has crossed in its row: 1 through `lanes`.
    let crossed = position % lanes + 1;
    let left_to_right = shuttle_row.is_multiple_of(2);

    let mut grid = Vec::with_capacity(rows);
    for row in 0..rows {
        // Half-open range of lanes in this row that already carry weft.
        let (start, end) = if row < shuttle_row {
            (0, lanes)
        } else if row > shuttle_row {
            (0, 0)
        } else if left_to_right {
            (0, crossed)
        } else {
            (lanes - crossed, lanes)
        };

        let leading_gap = match (row == shuttle_row, left_to_right) {
            (false, _) => None,
            (true, true) => Some(end),
            (true, false) => Some(start),
        };

        let mut line = String::with_capacity(lanes * 2);
        for lane in 0..lanes {
            let woven = (start..end).contains(&lane);
            line.push(if woven { '┼' } else { '│' });

            if lane + 1 < lanes {
                let next_woven = (start..end).contains(&(lane + 1));
                let travelling = leading_gap == Some(lane + 1);
                line.push(if (woven && next_woven) || travelling {
                    '─'
                } else {
                    ' '
                });
            }
        }
        grid.push(line);
    }
    grid
}

fn weave_spans(row: &str) -> Vec<Span<'static>> {
    row.chars()
        .enumerate()
        .map(|(index, ch)| {
            let style = if index.is_multiple_of(2) {
                Style::default().fg(lane_color(index / 2))
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Span::styled(ch.to_string(), style)
        })
        .collect()
}

/// A bar `width` cells wide, plus its end caps. A known `ratio` fills
/// proportionally; without one a block sweeps back and forth.
pub fn progress_bar(width: usize, ratio: Option<f64>, frame: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut bar = String::with_capacity(width + 2);
    bar.push('▕');
    match ratio {
        Some(ratio) => {
            let filled = ((ratio.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
            bar.push_str(&"█".repeat(filled));
            bar.push_str(&"░".repeat(width - filled));
        }
        None => {
            let block = (width / 4).max(1);
            let travel = width - block;
            let start = if travel == 0 {
                0
            } else {
                let phase = frame % (travel * 2);
                if phase <= travel {
                    phase
                } else {
                    travel * 2 - phase
                }
            };
            for index in 0..width {
                bar.push(if (start..start + block).contains(&index) {
                    '█'
                } else {
                    '░'
                });
            }
        }
    }
    bar.push('▏');
    bar
}

/// The line under the bar: stage progress and elapsed time.
fn counters(load: &LoadingState) -> String {
    let elapsed = format_elapsed(load.elapsed());
    match (load.done, load.total) {
        (0, _) => elapsed,
        (done, None) => format!("{} commits · {}", thousands(done), elapsed),
        (done, Some(total)) => format!(
            "{} / {} commits · {}",
            thousands(done),
            thousands(total),
            elapsed
        ),
    }
}

fn percent(ratio: f64) -> u64 {
    (ratio.clamp(0.0, 1.0) * 100.0).round() as u64
}

pub fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{:.1}s", elapsed.as_secs_f64())
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

pub fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn center(width: usize, spans: Vec<Span<'static>>) -> Line<'static> {
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = width.saturating_sub(used) / 2;
    if pad == 0 {
        return Line::from(spans);
    }

    let mut padded = Vec::with_capacity(spans.len() + 1);
    padded.push(Span::raw(" ".repeat(pad)));
    padded.extend(spans);
    Line::from(padded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weave_starts_bare_with_the_shuttle_entering_from_the_left() {
        let grid = weave_rows(0, 6, 4);
        assert_eq!(grid.len(), 4);
        assert_eq!(grid[0], "┼─│ │ │ │ │");
        assert_eq!(grid[1], "│ │ │ │ │ │");
        assert_eq!(grid[3], "│ │ │ │ │ │");
    }

    #[test]
    fn a_finished_row_is_fully_woven() {
        assert_eq!(weave_rows(5, 6, 4)[0], "┼─┼─┼─┼─┼─┼");
    }

    #[test]
    fn the_shuttle_reverses_on_the_next_row() {
        let grid = weave_rows(6, 6, 4);
        assert_eq!(grid[0], "┼─┼─┼─┼─┼─┼", "cloth stays woven");
        assert_eq!(grid[1], "│ │ │ │ │─┼", "entered from the right");
    }

    #[test]
    fn every_row_keeps_its_width_through_several_cycles() {
        for frame in 0..(6 * 4 * 3) {
            for row in weave_rows(frame, 6, 4) {
                assert_eq!(row.chars().count(), 11, "frame {frame}");
            }
        }
    }

    #[test]
    fn the_pattern_repeats_once_the_cloth_is_finished() {
        assert_eq!(weave_rows(0, 6, 4), weave_rows(24, 6, 4));
    }

    #[test]
    fn weave_is_empty_when_there_is_nothing_to_draw() {
        assert!(weave_rows(3, 0, 4).is_empty());
        assert!(weave_rows(3, 6, 0).is_empty());
    }

    #[test]
    fn determinate_bar_fills_in_proportion() {
        assert_eq!(progress_bar(4, Some(0.0), 0), "▕░░░░▏");
        assert_eq!(progress_bar(4, Some(0.5), 0), "▕██░░▏");
        assert_eq!(progress_bar(4, Some(1.0), 0), "▕████▏");
        assert_eq!(progress_bar(4, Some(2.0), 0), "▕████▏", "clamped");
    }

    #[test]
    fn indeterminate_bar_sweeps_without_changing_width() {
        let bar = progress_bar(12, None, 0);
        assert_eq!(bar.chars().count(), 14);
        assert_eq!(bar.chars().filter(|c| *c == '█').count(), 3);
        assert_ne!(progress_bar(12, None, 4), bar, "the block moves");
        assert_eq!(progress_bar(12, None, 18), bar, "and bounces back");
    }

    #[test]
    fn a_zero_width_bar_draws_nothing() {
        assert!(progress_bar(0, None, 7).is_empty());
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1024), "1,024");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn elapsed_switches_to_minutes_after_a_minute() {
        assert_eq!(format_elapsed(Duration::from_millis(4200)), "4.2s");
        assert_eq!(format_elapsed(Duration::from_secs(64)), "1m 04s");
    }

    #[test]
    fn centering_pads_the_left_and_never_overflows() {
        let line = center(11, vec![Span::raw("abc")]);
        assert_eq!(line.spans[0].content, "    ");
        let wide = center(2, vec![Span::raw("abcdef")]);
        assert_eq!(wide.spans.len(), 1);
    }
}
