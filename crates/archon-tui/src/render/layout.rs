//! Layout computation for the TUI render pipeline.
//!
//! `Layout` holds the computed `Rect` regions for each UI area.
//! `compute_layout()` produces a `Layout` from the terminal size.

use ratatui::layout::{Constraint, Direction, Rect};

use crate::output::OutputBuffer;

pub const MIN_INPUT_HEIGHT: u16 = 3;
pub const MAX_INPUT_HEIGHT: u16 = 12;

/// Computed layout regions for the TUI.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Output area (top, takes remaining space).
    pub output: Rect,
    /// Input area (3 rows, just below output).
    pub input: Rect,
    /// Permission indicator (1 row).
    pub permission: Rect,
    /// Status bar (1 row, bottom).
    pub status: Rect,
}

/// Compute the layout regions for a terminal of the given size.
///
/// The layout is always:
///
/// ```text
/// ┌──────────────────────────────────────┐
/// │           OUTPUT AREA                │
/// │         (min 3 rows)                  │
/// ├──────────────┬───────────────────────┤
/// │  INPUT AREA  │                       │
/// │  (3-12 rows) │                       │
/// ├──────────────┴───────────────────────┤
/// │  PERMISSION (1 row)                  │
/// ├───────────────────────────────────────┤
/// │  STATUS (1 row)                      │
/// └───────────────────────────────────────┘
/// ```
pub fn compute_layout(size: Rect) -> Layout {
    compute_layout_with_input_height(size, MIN_INPUT_HEIGHT)
}

pub fn compute_layout_with_input_height(size: Rect, input_height: u16) -> Layout {
    let input_height = clamp_input_height(size, input_height);
    let chunks = ratatui::layout::Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),               // output area
            Constraint::Length(input_height), // input area
            Constraint::Length(1),            // permission indicator
            Constraint::Length(1),            // status bar
        ])
        .split(size);

    Layout {
        output: chunks[0],
        input: chunks[1],
        permission: chunks[2],
        status: chunks[3],
    }
}

pub fn input_height_for_display(size: Rect, display_text: &str) -> u16 {
    let rows = input_display_rows(display_text, size.width);
    clamp_input_height(size, rows.saturating_add(1))
}

pub fn input_display_rows(display_text: &str, width: u16) -> u16 {
    let lines: Vec<&str> = display_text.split('\n').collect();
    OutputBuffer::count_wrapped_rows(&lines, width).max(1)
}

pub fn input_scroll_for_cursor(cursor_row: u16, visible_rows: u16) -> u16 {
    if visible_rows == 0 {
        0
    } else {
        cursor_row.saturating_sub(visible_rows.saturating_sub(1))
    }
}

pub fn wrapped_cursor_position(text_before_cursor: &str, width: u16) -> (u16, u16) {
    if width == 0 {
        return (0, 0);
    }

    let mut row: u32 = 0;
    let mut col: usize = 0;
    let parts: Vec<&str> = text_before_cursor.split('\n').collect();
    let width = width.max(1) as usize;

    for (idx, part) in parts.iter().enumerate() {
        let (part_rows, part_col) = wrapped_line_cursor_position(part, width);
        if idx + 1 == parts.len() {
            row = row.saturating_add(part_rows.saturating_sub(1) as u32);
            col = part_col;
        } else {
            row = row.saturating_add(part_rows as u32);
            col = 0;
        }
    }

    (
        row.min(u16::MAX as u32) as u16,
        col.min(width.saturating_sub(1)) as u16,
    )
}

fn wrapped_line_cursor_position(line: &str, width: usize) -> (usize, usize) {
    if line.is_empty() {
        return (1, 0);
    }

    let mut row_width: usize = 0;
    let mut rows: usize = 1;

    for token in line.split_inclusive(' ') {
        let token_width = token.chars().count();

        if row_width == 0 {
            if token_width >= width {
                let extra = (token_width - 1) / width;
                rows += extra;
                row_width = token_width - extra * width;
            } else {
                row_width = token_width;
            }
        } else if row_width + token_width <= width {
            row_width += token_width;
        } else {
            rows += 1;
            if token_width >= width {
                let extra = (token_width - 1) / width;
                rows += extra;
                row_width = token_width - extra * width;
            } else {
                row_width = token_width;
            }
        }
    }

    (rows, row_width)
}

fn clamp_input_height(size: Rect, desired_height: u16) -> u16 {
    let terminal_cap = size.height.saturating_sub(5).max(MIN_INPUT_HEIGHT);
    desired_height
        .max(MIN_INPUT_HEIGHT)
        .min(MAX_INPUT_HEIGHT)
        .min(terminal_cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_height_stays_compact_for_short_prompt() {
        let area = Rect::new(0, 0, 80, 24);
        assert_eq!(input_height_for_display(area, "> hello"), MIN_INPUT_HEIGHT);
    }

    #[test]
    fn input_height_expands_for_wrapped_prompt() {
        let area = Rect::new(0, 0, 24, 24);
        let long = format!("> {}", "word ".repeat(24));
        let height = input_height_for_display(area, &long);
        assert!(height > MIN_INPUT_HEIGHT, "height={height}");
        assert!(height <= MAX_INPUT_HEIGHT, "height={height}");
    }

    #[test]
    fn input_height_leaves_room_for_output_and_chrome() {
        let area = Rect::new(0, 0, 20, 10);
        let long = format!("> {}", "word ".repeat(80));
        let height = input_height_for_display(area, &long);
        assert!(height <= 5, "height={height}");
    }

    #[test]
    fn input_scroll_keeps_cursor_visible_at_bottom() {
        assert_eq!(input_scroll_for_cursor(0, 4), 0);
        assert_eq!(input_scroll_for_cursor(3, 4), 0);
        assert_eq!(input_scroll_for_cursor(4, 4), 1);
        assert_eq!(input_scroll_for_cursor(10, 4), 7);
    }

    #[test]
    fn wrapped_cursor_position_wraps_at_width() {
        assert_eq!(wrapped_cursor_position("> hi", 10), (0, 4));
        assert_eq!(wrapped_cursor_position("abcdef", 3), (1, 2));
        assert_eq!(wrapped_cursor_position("ab\ncd", 10), (1, 2));
    }

    #[test]
    fn wrapped_cursor_position_matches_word_wrap_tail_row() {
        let input = format!("> {} TAIL_MARKER", "long pasted prompt ".repeat(80));
        let (row, _) = wrapped_cursor_position(&input, 100);
        let display_rows = input_display_rows(&input, 100);
        assert_eq!(row, display_rows - 1);
    }
}
