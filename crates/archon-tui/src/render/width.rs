//! Display-width helpers for width-exact rendering.
//!
//! A terminal advances its cursor by a grapheme's *display width*, never by
//! its byte length or its `char` count. Every place the TUI derives a column
//! from a string — the status bar, the input line's wrapped-cursor maths, the
//! session badge — has to use that same measure. When it does not, the width
//! that was written diverges from the width that was computed, ratatui's frame
//! diff stops agreeing with the cells the terminal actually painted, and the
//! leftovers show up as stray glyphs (#174 part 2, point 3).
//!
//! `unicode-width` is the measure ratatui itself uses in `Buffer::set_stringn`
//! and in its reflow code, so agreeing with this module is agreeing with the
//! renderer.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Number of terminal columns `text` occupies.
pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// The longest grapheme-boundary prefix of `text` that fits in `width` columns.
///
/// A double-width grapheme that would straddle the limit is dropped rather
/// than half-written — writing it would push the terminal cursor one column
/// past where the caller believes it is.
pub fn truncate_to_width(text: &str, width: usize) -> String {
    let mut used = 0usize;
    let mut out = String::with_capacity(text.len().min(width.saturating_mul(4)));
    for grapheme in text.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if used + grapheme_width > width {
            break;
        }
        used += grapheme_width;
        out.push_str(grapheme);
    }
    out
}

/// `text` truncated to `width` columns and space-padded to occupy exactly
/// `width` columns.
///
/// Padding matters as much as truncation: a row rendered to its full width is
/// a row of cells the widget owns, so nothing that was there before can show
/// through, and the trailing style is the widget's own rather than whatever
/// the previous frame left.
pub fn fit_to_width(text: &str, width: usize) -> String {
    let mut out = truncate_to_width(text, width);
    let padding = width.saturating_sub(display_width(&out));
    out.extend(std::iter::repeat_n(' ', padding));
    out
}

/// Wrap a token that is at least as wide as a row, character-wrapping style.
///
/// Returns `(extra_rows, tail_width)`: how many row breaks the token forces
/// beyond the row it starts on, and how many columns it occupies on the last
/// of them.
///
/// Column arithmetic (`(token_width - 1) / width`) is only right while every
/// character is one column wide. A grapheme is atomic on screen — a
/// double-width one cannot straddle the right margin, so it moves to the next
/// row and leaves the previous one a column short. Walking graphemes is the
/// only way to land on the same column the terminal does.
pub fn split_across_rows(token: &str, width: usize) -> (usize, usize) {
    if width == 0 {
        return (0, 0);
    }
    let mut extra_rows = 0usize;
    let mut row_width = 0usize;
    for grapheme in token.graphemes(true) {
        // A grapheme wider than a whole row cannot be placed honestly; charge
        // it a full row so the caller's column count stays bounded.
        let grapheme_width = UnicodeWidthStr::width(grapheme).min(width);
        if row_width + grapheme_width > width {
            extra_rows += 1;
            row_width = 0;
        }
        row_width += grapheme_width;
    }
    (extra_rows, row_width)
}

#[cfg(test)]
mod tests {
    use super::{display_width, fit_to_width, split_across_rows, truncate_to_width};

    #[test]
    fn display_width_counts_columns_not_chars() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("世界"), 4);
        assert_eq!(display_width("e\u{301}"), 1);
    }

    #[test]
    fn truncate_drops_a_double_width_grapheme_that_would_straddle_the_limit() {
        // "世" needs two columns; with one column left it must not be written
        // at all, or the terminal cursor ends up one column past the model.
        assert_eq!(truncate_to_width("a世", 2), "a");
        assert_eq!(truncate_to_width("a世", 3), "a世");
        assert_eq!(truncate_to_width("世界", 4), "世界");
    }

    #[test]
    fn fit_pads_to_exactly_the_requested_column_count() {
        assert_eq!(display_width(&fit_to_width("abc", 6)), 6);
        assert_eq!(display_width(&fit_to_width("世界", 5)), 5);
        assert_eq!(display_width(&fit_to_width("世界世", 5)), 5);
        assert_eq!(fit_to_width("abcdef", 3), "abc");
    }

    #[test]
    fn fit_to_zero_width_writes_nothing() {
        assert_eq!(fit_to_width("anything", 0), "");
    }

    #[test]
    fn split_across_rows_matches_column_arithmetic_for_narrow_text() {
        // The behaviour this replaced, preserved exactly where every character
        // is one column wide.
        for (token, width) in [("abcdef", 3), ("abc", 3), ("abcdefg", 3), ("ab", 5)] {
            let token_width = display_width(token);
            let expected = if token_width >= width {
                let extra = (token_width - 1) / width;
                (extra, token_width - extra * width)
            } else {
                (0, token_width)
            };
            assert_eq!(split_across_rows(token, width), expected, "{token}/{width}");
        }
    }

    #[test]
    fn split_across_rows_never_straddles_a_double_width_grapheme() {
        // Three double-width graphemes in five columns: two fit, the third
        // moves down and the first row ends one column short.
        assert_eq!(split_across_rows("世界世", 5), (1, 2));
        assert_eq!(split_across_rows("世界", 4), (0, 4));
        assert_eq!(split_across_rows("世界世界", 4), (1, 4));
    }

    #[test]
    fn split_across_rows_tolerates_a_grapheme_wider_than_the_row() {
        assert_eq!(split_across_rows("世", 1), (0, 1));
        assert_eq!(split_across_rows("anything", 0), (0, 0));
    }
}
