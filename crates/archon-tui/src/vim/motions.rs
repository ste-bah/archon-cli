//! Cursor movement primitives for `VimState`.
//!
//! Relocated from `src/vim.rs` (movement section L568-L717 + helpers
//! L955-L984) per REM-2e. All `move_*` entry points take a count; the
//! `_once` helpers and clamp/size utilities stay in this module.

use super::{VimMode, VimState};

impl VimState {
    // -- movement -----------------------------------------------------------

    pub(super) fn move_left(&mut self, count: usize) {
        for _ in 0..count {
            if self.cursor_col > 0 {
                self.cursor_col -= 1;
            }
        }
    }

    pub(super) fn move_right(&mut self, count: usize) {
        let max_col = self.max_cursor_col();
        for _ in 0..count {
            if self.cursor_col < max_col {
                self.cursor_col += 1;
            }
        }
    }

    pub(super) fn move_up(&mut self, count: usize) {
        for _ in 0..count {
            if self.cursor_row > 0 {
                self.cursor_row -= 1;
            }
        }
        self.clamp_col();
    }

    pub(super) fn move_down(&mut self, count: usize) {
        for _ in 0..count {
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
            }
        }
        self.clamp_col();
    }

    pub(super) fn move_word_forward(&mut self, count: usize) {
        for _ in 0..count {
            self.move_word_forward_once();
        }
    }

    fn move_word_forward_once(&mut self) {
        let line = &self.lines[self.cursor_row];
        let len = line.len();
        if self.cursor_col >= len {
            // Move to next line if possible
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                self.cursor_col = 0;
                // Skip leading whitespace on new line
                let next_line = &self.lines[self.cursor_row];
                let first_non_ws = next_line.find(|c: char| !c.is_whitespace()).unwrap_or(0);
                self.cursor_col = first_non_ws;
            }
            return;
        }

        let bytes = line.as_bytes();
        let mut col = self.cursor_col;

        // Skip current word (non-whitespace)
        while col < len && !bytes[col].is_ascii_whitespace() {
            col += 1;
        }
        // Skip whitespace
        while col < len && bytes[col].is_ascii_whitespace() {
            col += 1;
        }

        if col >= len {
            // Went past end of line — try next line
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                let next_line = &self.lines[self.cursor_row];
                self.cursor_col = next_line.find(|c: char| !c.is_whitespace()).unwrap_or(0);
            } else {
                self.cursor_col = len.saturating_sub(1);
            }
        } else {
            self.cursor_col = col;
        }
    }

    pub(super) fn move_word_backward(&mut self, count: usize) {
        for _ in 0..count {
            self.move_word_backward_once();
        }
    }

    fn move_word_backward_once(&mut self) {
        if self.cursor_col == 0 {
            // Move to end of previous line
            if self.cursor_row > 0 {
                self.cursor_row -= 1;
                self.cursor_col = self.current_line_len().saturating_sub(1);
            }
            return;
        }

        let line = &self.lines[self.cursor_row];
        let bytes = line.as_bytes();
        let mut col = self.cursor_col;

        // Skip whitespace backwards
        while col > 0 && bytes[col - 1].is_ascii_whitespace() {
            col -= 1;
        }
        // Skip word backwards
        while col > 0 && !bytes[col - 1].is_ascii_whitespace() {
            col -= 1;
        }

        self.cursor_col = col;
    }

    pub(super) fn move_word_end(&mut self, count: usize) {
        for _ in 0..count {
            self.move_word_end_once();
        }
    }

    fn move_word_end_once(&mut self) {
        let line = &self.lines[self.cursor_row];
        let len = line.len();
        if len == 0 {
            return;
        }
        let bytes = line.as_bytes();
        let mut col = self.cursor_col + 1;

        // Skip whitespace
        while col < len && bytes[col].is_ascii_whitespace() {
            col += 1;
        }
        // Move to end of word
        while col + 1 < len && !bytes[col + 1].is_ascii_whitespace() {
            col += 1;
        }

        self.cursor_col = col.min(len.saturating_sub(1));
    }

    pub(super) fn move_line_start(&mut self) {
        self.cursor_col = 0;
    }

    pub(super) fn move_line_end(&mut self) {
        let len = self.current_line_len();
        self.cursor_col = if len > 0 { len - 1 } else { 0 };
    }

    // -- helpers ------------------------------------------------------------

    pub(super) fn current_line_len(&self) -> usize {
        self.lines.get(self.cursor_row).map_or(0, |l| l.len())
    }

    /// Maximum column for cursor in current mode.
    pub(super) fn max_cursor_col(&self) -> usize {
        let len = self.current_line_len();
        match self.mode {
            VimMode::Insert => len,
            _ => {
                if len > 0 {
                    len - 1
                } else {
                    0
                }
            }
        }
    }

    pub(super) fn clamp_col(&mut self) {
        let max = self.max_cursor_col();
        if self.cursor_col > max {
            self.cursor_col = max;
        }
    }

    /// Consume and return the pending count prefix (defaults to 1).
    pub(super) fn take_count(&mut self) -> usize {
        self.pending_count.take().unwrap_or(1)
    }
}
