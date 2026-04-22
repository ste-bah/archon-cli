//! Append-only streaming output buffer with scroll and word-wrap math.
//!
//! Relocated from `src/output.rs` (OutputBuffer section, L210-L374 + tests
//! L380-L495) per REM-2h.

/// Output buffer -- append-only text buffer for streaming display.
#[derive(Debug, Default)]
pub struct OutputBuffer {
    lines: Vec<String>,
    current_line: String,
    /// Vertical scroll offset (lines from the top). When `scroll_locked` is
    /// false this is ignored and we auto-scroll to the bottom.
    pub scroll_offset: u16,
    /// When true the user has scrolled away from the bottom; new content does
    /// not auto-scroll.
    pub scroll_locked: bool,
}

impl OutputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append text (may contain newlines).
    pub fn append(&mut self, text: &str) {
        for ch in text.chars() {
            if ch == '\n' {
                self.lines.push(std::mem::take(&mut self.current_line));
            } else {
                self.current_line.push(ch);
            }
        }
    }

    /// Append a complete line.
    pub fn append_line(&mut self, line: &str) {
        if !self.current_line.is_empty() {
            self.lines.push(std::mem::take(&mut self.current_line));
        }
        self.lines.push(line.to_string());
    }

    /// Get all completed lines plus the current partial line.
    pub fn all_lines(&self) -> Vec<&str> {
        let mut result: Vec<&str> = self.lines.iter().map(|s| s.as_str()).collect();
        if !self.current_line.is_empty() {
            result.push(&self.current_line);
        }
        result
    }

    /// Total line count (including partial current line).
    pub fn line_count(&self) -> usize {
        self.lines.len() + if self.current_line.is_empty() { 0 } else { 1 }
    }

    /// Clear all content.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.current_line.clear();
        self.scroll_offset = 0;
        self.scroll_locked = false;
    }

    // -- scroll helpers -----------------------------------------------------

    /// Scroll up by `amount` lines (see earlier content). Locks auto-scroll.
    /// `scroll_offset` = lines scrolled UP from the bottom.
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.scroll_locked = true;
    }

    /// Scroll down by `amount` lines (toward newer content).
    /// If offset reaches 0, unlocks auto-scroll.
    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        if self.scroll_offset == 0 {
            self.scroll_locked = false;
        }
    }

    /// Jump to the bottom and unlock auto-scroll.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.scroll_locked = false;
    }

    /// Compute the actual scroll position for the `Paragraph::scroll()` call.
    ///
    /// `scroll_offset` = lines scrolled UP from the bottom.
    /// `Paragraph::scroll((y, 0))` expects physical rows from the TOP.
    /// NOTE: ratatui does NOT clamp — passing a value past content shows blank.
    ///
    /// When not scroll-locked: auto-scroll to bottom (return max_scroll).
    /// When locked: return max_scroll - scroll_offset (clamped to zero).
    pub fn effective_scroll(&self, total_wrapped_rows: u16, visible_height: u16) -> u16 {
        let max_scroll = total_wrapped_rows.saturating_sub(visible_height);
        if !self.scroll_locked {
            max_scroll
        } else {
            max_scroll.saturating_sub(self.scroll_offset)
        }
    }

    /// Count total wrapped rows given a terminal width using word-wrap simulation.
    ///
    /// Matches ratatui's `Wrap { trim: false }` behaviour: words are kept together
    /// where possible; only overlong single words are char-split. Simple
    /// `ceil(len/width)` underestimates when words don't align to line boundaries,
    /// causing the scroll position to fall short of the actual bottom.
    pub fn count_wrapped_rows(lines: &[&str], width: u16) -> u16 {
        if width == 0 {
            return lines.len() as u16;
        }
        let w = width as usize;
        let mut total: u32 = 0;

        for &line in lines {
            if line.is_empty() {
                total += 1;
                continue;
            }

            // Fast path: whole line fits on one row
            let char_count = line.chars().count();
            if char_count <= w {
                total += 1;
                continue;
            }

            // Word-wrap: split_inclusive keeps the space attached to the
            // preceding word ("hello ", "world"), mirroring ratatui's greedy fit.
            let mut row_width: usize = 0;
            let mut rows: u32 = 1;

            for token in line.split_inclusive(' ') {
                let token_w = token.chars().count();

                if row_width == 0 {
                    // Start of a new row
                    if token_w >= w {
                        // Single token wider than the row — char-wrap it
                        let extra = (token_w - 1) / w;
                        rows += extra as u32;
                        row_width = token_w - extra * w;
                    } else {
                        row_width = token_w;
                    }
                } else if row_width + token_w <= w {
                    row_width += token_w;
                } else {
                    // Token doesn't fit — start a new row
                    rows += 1;
                    if token_w >= w {
                        let extra = (token_w - 1) / w;
                        rows += extra as u32;
                        row_width = token_w - extra * w;
                    } else {
                        row_width = token_w;
                    }
                }
            }

            total += rows;
        }

        total.min(u16::MAX as u32) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_text_with_newlines() {
        let mut buf = OutputBuffer::new();
        buf.append("hello\nworld\n");
        assert_eq!(buf.all_lines(), vec!["hello", "world"]);
    }

    #[test]
    fn append_streaming_chars() {
        let mut buf = OutputBuffer::new();
        buf.append("H");
        buf.append("e");
        buf.append("l");
        buf.append("lo");
        assert_eq!(buf.all_lines(), vec!["Hello"]);
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn append_line() {
        let mut buf = OutputBuffer::new();
        buf.append_line("first");
        buf.append_line("second");
        assert_eq!(buf.all_lines(), vec!["first", "second"]);
    }

    // -- scroll tests -------------------------------------------------------

    #[test]
    fn scroll_up_locks_and_increases_offset() {
        let mut buf = OutputBuffer::new();
        // scroll_offset = lines scrolled UP from bottom. scroll_up adds.
        buf.scroll_up(5);
        assert!(buf.scroll_locked);
        assert_eq!(buf.scroll_offset, 5);
        buf.scroll_up(3);
        assert_eq!(buf.scroll_offset, 8);
    }

    #[test]
    fn scroll_down_decreases_offset() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 10;
        buf.scroll_down(3);
        assert_eq!(buf.scroll_offset, 7);
        assert!(buf.scroll_locked); // still locked, not at bottom
    }

    #[test]
    fn scroll_down_to_zero_unlocks() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 3;
        buf.scroll_down(5); // saturating_sub: 3 - 5 = 0
        assert_eq!(buf.scroll_offset, 0);
        assert!(!buf.scroll_locked); // reached bottom, unlocked
    }

    #[test]
    fn scroll_to_bottom_resets() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 10;
        buf.scroll_to_bottom();
        assert_eq!(buf.scroll_offset, 0);
        assert!(!buf.scroll_locked);
    }

    #[test]
    fn effective_scroll_at_bottom() {
        let buf = OutputBuffer::new();
        // Not locked => auto-scroll to bottom => max_scroll = 30 - 10 = 20
        assert_eq!(buf.effective_scroll(30, 10), 20);
    }

    #[test]
    fn count_wrapped_rows_word_wrap_differs_from_char_wrap() {
        // "hi hello world" = 14 chars, width 7
        // Simple ceil(14/7) = 2 — WRONG for word-wrap
        // Word-wrap: "hi " (3) fits row1, "hello " (6) overflows → row2,
        //            "world" (5) overflows row2(6+5=11>7) → row3 = 3 rows
        let lines = ["hi hello world"];
        assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 7), 3);
    }

    #[test]
    fn count_wrapped_rows_long_word_char_splits() {
        // "abcdefghijklmnop" = 17 chars, width 5 → ceil(17/5) = 4 rows
        let lines = ["abcdefghijklmnop"];
        assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 5), 4);
    }

    #[test]
    fn count_wrapped_rows_fits_on_one_row() {
        let lines = ["hello world"];
        assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 20), 1);
    }

    #[test]
    fn effective_scroll_scrolled_up() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 5;
        // max_scroll = 30 - 10 = 20. effective = 20 - 5 = 15 (scrolled 5 lines up from bottom)
        assert_eq!(buf.effective_scroll(30, 10), 15);
    }

    #[test]
    fn effective_scroll_clamped_to_zero() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 100; // way past content
        // max_scroll = 30 - 10 = 20. effective = 20 - 100 = 0 (clamped via saturating_sub)
        assert_eq!(buf.effective_scroll(30, 10), 0);
    }
}
