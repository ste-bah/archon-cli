//! Layout computation for the TUI render pipeline.
//!
//! `Layout` holds the computed `Rect` regions for each UI area.
//! `compute_layout()` produces a `Layout` from the terminal size.

use ratatui::layout::{Constraint, Direction, Rect};

/// Rows of the input region taken by chrome rather than draft text — the
/// single `Borders::TOP` rule drawn by `body::draw_input_area`.
const INPUT_CHROME_ROWS: u16 = 1;

/// How many rows of draft the input area will grow to before it starts
/// scrolling internally (issue #174). Eight is deep enough to hold a
/// paragraph-sized prompt while still leaving the transcript the majority of
/// an 80x24 terminal.
pub const MAX_INPUT_TEXT_ROWS: u16 = 8;

pub const MIN_INPUT_HEIGHT: u16 = 5;
pub const MAX_INPUT_HEIGHT: u16 = MAX_INPUT_TEXT_ROWS + INPUT_CHROME_ROWS;

/// Computed layout regions for the TUI.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Output area (top, takes remaining space).
    pub output: Rect,
    /// Input area (`MIN_INPUT_HEIGHT`..=`MAX_INPUT_HEIGHT` rows, just below
    /// output).
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
/// │  (5-9 rows)  │                       │
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

/// Height the input region needs to show `display_text`, chrome included.
///
/// Grows with the draft up to [`MAX_INPUT_TEXT_ROWS`] rows of text; past that
/// the region stops growing and [`input_scroll_for_cursor`] scrolls inside it
/// (issue #174).
pub fn input_height_for_display(size: Rect, display_text: &str) -> u16 {
    let rows = input_display_rows(display_text, size.width);
    clamp_input_height(size, rows.saturating_add(INPUT_CHROME_ROWS))
}

/// Rows the draft occupies once wrapped — embedded newlines plus soft wraps.
///
/// Measured with [`wrapped_line_cursor_position`], the same word-wrap model
/// [`wrapped_cursor_position`] uses. They have to agree: the height decides
/// how many rows are visible and the cursor position decides which row to
/// scroll to, so measuring them differently (this used to count *character*
/// wraps via `OutputBuffer::count_wrapped_rows`) makes a word-wrapped draft
/// scroll its own first line out of view while there is still room for it.
pub fn input_display_rows(display_text: &str, width: u16) -> u16 {
    if width == 0 {
        return display_text.split('\n').count().clamp(1, u16::MAX as usize) as u16;
    }
    let width = width.max(1) as usize;
    display_text
        .split('\n')
        .map(|line| wrapped_line_cursor_position(line, width).0)
        .sum::<usize>()
        .clamp(1, u16::MAX as usize) as u16
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
        // Display width, not `chars().count()`: this row count feeds both
        // `input_display_rows` and `wrapped_cursor_position`, and the column
        // it returns is handed straight to the terminal cursor. Counting chars
        // put the cursor one column left of the glyph for every double-width
        // character on the line (#174 part 2, point 3).
        let token_width = super::width::display_width(token);

        if row_width == 0 {
            (rows, row_width) = start_token_on_empty_row(token, token_width, width, rows);
        } else if row_width + token_width <= width {
            row_width += token_width;
        } else {
            rows += 1;
            (rows, row_width) = start_token_on_empty_row(token, token_width, width, rows);
        }
    }

    (rows, row_width)
}

/// Place `token` at the start of an empty row, character-wrapping it when it
/// is too wide to fit, and report the running row count and the columns used
/// on the final row.
fn start_token_on_empty_row(
    token: &str,
    token_width: usize,
    width: usize,
    rows: usize,
) -> (usize, usize) {
    if token_width < width {
        return (rows, token_width);
    }
    let (extra_rows, tail_width) = super::width::split_across_rows(token, width);
    (rows + extra_rows, tail_width)
}

fn clamp_input_height(size: Rect, desired_height: u16) -> u16 {
    let terminal_cap = size.height.saturating_sub(5).max(MIN_INPUT_HEIGHT);
    desired_height
        .clamp(MIN_INPUT_HEIGHT, MAX_INPUT_HEIGHT)
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
    fn wrapped_cursor_position_measures_double_width_characters_in_columns() {
        // "世界" paints four columns, so the cursor after it sits at column 4.
        assert_eq!(wrapped_cursor_position("世界", 10), (0, 4));
        // ...and the line wraps two columns earlier than a char count implies.
        assert_eq!(wrapped_cursor_position("世界世", 5), (1, 2));
    }

    #[test]
    fn wrapped_cursor_row_agrees_with_input_display_rows_on_wide_text() {
        let input = format!("> {}", "世界 ".repeat(30));
        let (row, _) = wrapped_cursor_position(&input, 40);
        assert_eq!(row, input_display_rows(&input, 40) - 1);
    }

    #[test]
    fn wrapped_cursor_position_matches_word_wrap_tail_row() {
        let input = format!("> {} TAIL_MARKER", "long pasted prompt ".repeat(80));
        let (row, _) = wrapped_cursor_position(&input, 100);
        let display_rows = input_display_rows(&input, 100);
        assert_eq!(row, display_rows - 1);
    }

    // ── Multi-line drafts (issue #174) ────────────────────────────────────

    /// A draft with embedded newlines occupies one row per line even when no
    /// line is long enough to wrap — the case Shift+Enter / Alt+Enter creates.
    #[test]
    fn embedded_newlines_each_take_a_row() {
        assert_eq!(input_display_rows("> one\ntwo\nthree", 80), 3);
        assert_eq!(
            input_display_rows("> one\n\nthree", 80),
            3,
            "blank lines too"
        );
        assert_eq!(input_display_rows("> trailing\n", 80), 2);
    }

    #[test]
    fn input_area_grows_one_row_per_added_line() {
        let area = Rect::new(0, 0, 80, 40);
        // MIN_INPUT_HEIGHT already covers the first four rows of draft, so
        // growth only becomes visible on the fifth.
        assert_eq!(input_height_for_display(area, "> a\nb\nc\nd"), 5);
        assert_eq!(input_height_for_display(area, "> a\nb\nc\nd\ne"), 6);
        assert_eq!(input_height_for_display(area, "> a\nb\nc\nd\ne\nf"), 7);
    }

    /// Growth stops at `MAX_INPUT_TEXT_ROWS`; beyond that the draft scrolls
    /// inside a fixed region rather than eating the transcript.
    #[test]
    fn input_area_stops_growing_at_the_text_row_cap() {
        let area = Rect::new(0, 0, 80, 40);
        let nine_lines = format!("> {}", "line\n".repeat(9));
        assert!(input_display_rows(&nine_lines, 80) > MAX_INPUT_TEXT_ROWS);
        assert_eq!(
            input_height_for_display(area, &nine_lines),
            MAX_INPUT_HEIGHT
        );

        let fifty_lines = format!("> {}", "line\n".repeat(50));
        assert_eq!(
            input_height_for_display(area, &fifty_lines),
            MAX_INPUT_HEIGHT
        );
    }

    /// The scroll offset must keep the cursor's row on screen once the draft
    /// is taller than the region.
    #[test]
    fn multiline_draft_scrolls_internally_to_follow_the_cursor() {
        let visible_rows = MAX_INPUT_HEIGHT - 1;
        let draft = format!("> {}last", "line\n".repeat(12));
        let (cursor_row, _) = wrapped_cursor_position(&draft, 80);
        assert_eq!(cursor_row, 12, "cursor sits on the thirteenth row");

        let scroll = input_scroll_for_cursor(cursor_row, visible_rows);
        assert_eq!(scroll, cursor_row - (visible_rows - 1));
        assert!(
            cursor_row - scroll < visible_rows,
            "cursor row {cursor_row} must land inside {visible_rows} visible rows at scroll {scroll}"
        );
    }

    /// Height and cursor row are two halves of the same decision, so they must
    /// be measured with the same wrap model. Word wrap pushes "hello" onto its
    /// own row where character wrap would have packed it — measuring the
    /// height with the latter used to scroll a draft that in fact fitted.
    #[test]
    fn height_and_cursor_agree_on_word_wrapped_drafts() {
        // "> hi " / "hello " / "world" — three rows word-wrapped, but only
        // two if you count characters (16 chars at width 9).
        let draft = "> hi hello world";
        assert_eq!(input_display_rows(draft, 9), 3);
        let (cursor_row, _) = wrapped_cursor_position(draft, 9);
        assert_eq!(cursor_row, 2);
    }
}
