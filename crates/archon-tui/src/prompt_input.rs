//! Multiline text buffer for the prompt input area.
//! Layer 1 module — no imports from screens/ or app/.

/// A multiline text buffer that tracks cursor position across multiple lines.
#[derive(Debug)]
pub struct PromptBuffer {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
}

impl PromptBuffer {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.is_empty())
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn insert_char(&mut self, c: char) {
        if c == '\n' {
            self.insert_newline();
            return;
        }
        let row = self.cursor_row;
        self.lines[row].insert(self.cursor_col, c);
        self.cursor_col += 1;
    }

    pub fn insert_newline(&mut self) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        let current = self.lines[row].clone();
        let remaining = current[col..].to_string();
        self.lines[row] = current[..col].to_string();
        self.lines.insert(row + 1, remaining);
        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            // Normal backspace: delete char before cursor
            let row = self.cursor_row;
            self.lines[row].remove(self.cursor_col - 1);
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            // At col 0 of non-first row: join current line with previous line
            let current_line = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&current_line);
        }
        // At (0, 0): nothing to backspace
    }

    pub fn delete(&mut self) {
        let row = self.cursor_row;
        if self.cursor_col < self.lines[row].len() {
            self.lines[row].remove(self.cursor_col);
        } else if self.cursor_row + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
        }
    }

    pub fn move_right(&mut self) {
        let row = self.cursor_row;
        if self.cursor_col < self.lines[row].len() {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            // At end of line: move to start of next line
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
        // At end of last line: do nothing
    }

    pub fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
        }
    }

    pub fn submit(&mut self) -> String {
        let text = self.text();
        self.lines = vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
        text
    }
}

impl Default for PromptBuffer {
    fn default() -> Self {
        Self::new()
    }
}
