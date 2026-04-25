//! Buffer editing primitives, visual-selection helpers, and undo/redo for
//! `VimState`.
//!
//! Relocated from `src/vim.rs` (editing L721-L847 + visual helpers
//! L853-L917 + undo/redo L921-L951) per REM-2e.

use super::{UndoEntry, VimState};

impl VimState {
    // -- editing ------------------------------------------------------------

    pub(super) fn insert_char(&mut self, c: char) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        if row < self.lines.len() {
            self.lines[row].insert(col, c);
            self.cursor_col += c.len_utf8();
        }
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let row = self.cursor_row;
            let col = self.cursor_col;
            // Find the previous char boundary
            let prev_char_len = self.lines[row][..col]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_col -= prev_char_len;
            self.lines[row].remove(self.cursor_col);
        } else if self.cursor_row > 0 {
            // Join with previous line
            let current = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&current);
        }
    }

    pub(super) fn insert_newline(&mut self) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        let rest = self.lines[row][col..].to_string();
        self.lines[row].truncate(col);
        self.lines.insert(row + 1, rest);
        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    pub(super) fn delete_line(&mut self, count: usize) {
        let count = count.min(self.lines.len() - self.cursor_row);
        let deleted: Vec<String> = self
            .lines
            .drain(self.cursor_row..self.cursor_row + count)
            .collect();
        self.yank_buffer = deleted.join("\n");
        self.yank_is_linewise = true;

        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        if self.cursor_row >= self.lines.len() {
            self.cursor_row = self.lines.len() - 1;
        }
        self.clamp_col();
    }

    pub(super) fn yank_line(&mut self, count: usize) {
        let end = (self.cursor_row + count).min(self.lines.len());
        let yanked: Vec<&str> = self.lines[self.cursor_row..end]
            .iter()
            .map(|s| s.as_str())
            .collect();
        self.yank_buffer = yanked.join("\n");
        self.yank_is_linewise = true;
    }

    pub(super) fn paste_after(&mut self) {
        if self.yank_buffer.is_empty() {
            return;
        }
        if self.yank_is_linewise {
            let new_lines: Vec<String> = self.yank_buffer.lines().map(String::from).collect();
            let insert_at = self.cursor_row + 1;
            for (i, line) in new_lines.into_iter().enumerate() {
                self.lines.insert(insert_at + i, line);
            }
            self.cursor_row += 1;
            self.cursor_col = 0;
        } else {
            let row = self.cursor_row;
            let col = (self.cursor_col + 1).min(self.lines[row].len());
            self.lines[row].insert_str(col, &self.yank_buffer);
            self.cursor_col = col + self.yank_buffer.len() - 1;
        }
    }

    pub(super) fn paste_before(&mut self) {
        if self.yank_buffer.is_empty() {
            return;
        }
        if self.yank_is_linewise {
            let new_lines: Vec<String> = self.yank_buffer.lines().map(String::from).collect();
            let insert_at = self.cursor_row;
            for (i, line) in new_lines.into_iter().enumerate() {
                self.lines.insert(insert_at + i, line);
            }
            self.cursor_col = 0;
        } else {
            let row = self.cursor_row;
            let col = self.cursor_col;
            self.lines[row].insert_str(col, &self.yank_buffer);
        }
    }

    pub(super) fn delete_char(&mut self) {
        let row = self.cursor_row;
        let len = self.lines[row].len();
        if len > 0 && self.cursor_col < len {
            self.lines[row].remove(self.cursor_col);
            // Clamp cursor if we deleted the last char
            let new_len = self.lines[row].len();
            if new_len > 0 && self.cursor_col >= new_len {
                self.cursor_col = new_len - 1;
            }
        }
    }

    pub(super) fn replace_char(&mut self, c: char) {
        let row = self.cursor_row;
        let len = self.lines[row].len();
        if len > 0 && self.cursor_col < len {
            self.lines[row].remove(self.cursor_col);
            self.lines[row].insert(self.cursor_col, c);
        }
    }

    // -- visual selection helpers ---------------------------------------------

    /// Compute the (start, end) byte range for the current visual selection.
    /// Returns `(start_row, start_col, end_row, end_col)` where start <= end.
    fn visual_range(&self) -> (usize, usize, usize, usize) {
        let (sr, sc) = self
            .visual_start
            .unwrap_or((self.cursor_row, self.cursor_col));
        let (er, ec) = (self.cursor_row, self.cursor_col);
        if (sr, sc) <= (er, ec) {
            (sr, sc, er, ec)
        } else {
            (er, ec, sr, sc)
        }
    }

    /// Yank the visual selection into the yank buffer.
    pub(super) fn visual_yank(&mut self) {
        let (sr, sc, er, ec) = self.visual_range();
        if sr == er {
            // Single line selection
            let line = &self.lines[sr];
            let end = (ec + 1).min(line.len());
            self.yank_buffer = line[sc..end].to_string();
            self.yank_is_linewise = false;
        } else {
            // Multi-line selection
            let mut result = String::new();
            result.push_str(&self.lines[sr][sc..]);
            for row in (sr + 1)..er {
                result.push('\n');
                result.push_str(&self.lines[row]);
            }
            result.push('\n');
            let end = (ec + 1).min(self.lines[er].len());
            result.push_str(&self.lines[er][..end]);
            self.yank_buffer = result;
            self.yank_is_linewise = false;
        }
    }

    /// Delete the visual selection and place deleted text in yank buffer.
    pub(super) fn visual_delete(&mut self) {
        let (sr, sc, er, ec) = self.visual_range();
        if sr == er {
            // Single line deletion
            let line = &self.lines[sr];
            let end = (ec + 1).min(line.len());
            self.yank_buffer = line[sc..end].to_string();
            self.yank_is_linewise = false;
            self.lines[sr] = format!("{}{}", &line[..sc], &line[end..]);
        } else {
            // Multi-line deletion — yank first
            self.visual_yank();
            // Keep prefix of first line + suffix of last line
            let prefix = self.lines[sr][..sc].to_string();
            let end = (ec + 1).min(self.lines[er].len());
            let suffix = self.lines[er][end..].to_string();
            // Remove all lines in range, replace with merged line
            self.lines.drain(sr..=er);
            self.lines.insert(sr, format!("{prefix}{suffix}"));
        }
        self.cursor_row = sr;
        self.cursor_col = sc;
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.clamp_col();
    }

    // -- undo / redo --------------------------------------------------------

    pub(super) fn save_undo(&mut self) {
        self.undo_stack.push(UndoEntry {
            lines_before: self.lines.clone(),
            cursor_before: (self.cursor_row, self.cursor_col),
        });
    }

    pub(super) fn undo(&mut self) {
        if let Some(entry) = self.undo_stack.pop() {
            // Save current state to redo stack
            self.redo_stack.push(UndoEntry {
                lines_before: self.lines.clone(),
                cursor_before: (self.cursor_row, self.cursor_col),
            });
            self.lines = entry.lines_before;
            self.cursor_row = entry.cursor_before.0;
            self.cursor_col = entry.cursor_before.1;
        }
    }

    pub(super) fn redo(&mut self) {
        if let Some(entry) = self.redo_stack.pop() {
            self.undo_stack.push(UndoEntry {
                lines_before: self.lines.clone(),
                cursor_before: (self.cursor_row, self.cursor_col),
            });
            self.lines = entry.lines_before;
            self.cursor_row = entry.cursor_before.0;
            self.cursor_col = entry.cursor_before.1;
        }
    }
}
