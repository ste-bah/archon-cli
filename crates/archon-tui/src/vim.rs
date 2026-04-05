//! Vim-style keybinding state machine for the TUI input area.
//!
//! When `TuiConfig::vim_mode` is enabled, key events are routed through
//! [`VimState`] instead of the default [`InputHandler`](crate::input::InputHandler).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// TuiConfig (lives here so archon-tui stays self-contained; archon-core
// re-exports or embeds it as needed)
// ---------------------------------------------------------------------------

/// TUI-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct TuiConfig {
    /// Enable vim-style keybindings in the input area. Default: `false`.
    pub vim_mode: bool,
}

// ---------------------------------------------------------------------------
// Public enums
// ---------------------------------------------------------------------------

/// Current vim editing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    Command,
}

/// Action returned to the caller after processing a key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VimAction {
    /// No external action needed.
    None,
    /// User submitted input (`:w` + Enter).
    Submit,
    /// User requested quit (`:q` + Enter).
    Quit,
    /// Content changed; the host should redraw.
    Redraw,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

enum InsertPosition {
    AtCursor,
    AfterCursor,
    LineStart,
    LineEnd,
    NewLineBelow,
    NewLineAbove,
}

struct UndoEntry {
    lines_before: Vec<String>,
    cursor_before: (usize, usize),
}

// ---------------------------------------------------------------------------
// VimState
// ---------------------------------------------------------------------------

/// Full vim editing state machine operating on a multi-line buffer.
pub struct VimState {
    mode: VimMode,
    cursor_row: usize,
    cursor_col: usize,
    lines: Vec<String>,
    yank_buffer: String,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    pending_count: Option<usize>,
    visual_start: Option<(usize, usize)>,
    command_buffer: String,
    /// Tracks whether we have already saved an undo snapshot for the current
    /// insert session. Reset when leaving insert mode.
    insert_undo_saved: bool,
    /// Pending operator key (e.g. 'd' waiting for a second 'd').
    pending_operator: Option<char>,
    /// Whether the yank buffer contains a line-wise yank (vs. character-wise).
    yank_is_linewise: bool,
}

impl VimState {
    /// Create a new `VimState` with a single empty line.
    pub fn new() -> Self {
        Self {
            mode: VimMode::Normal,
            cursor_row: 0,
            cursor_col: 0,
            lines: vec![String::new()],
            yank_buffer: String::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_count: None,
            visual_start: None,
            command_buffer: String::new(),
            insert_undo_saved: false,
            pending_operator: None,
            yank_is_linewise: false,
        }
    }

    /// Create a `VimState` pre-loaded with `text`.
    pub fn from_text(text: &str) -> Self {
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.lines().map(String::from).collect()
        };
        Self {
            lines,
            ..Self::new()
        }
    }

    // -- public API ---------------------------------------------------------

    /// Process a key event and return the resulting action.
    pub fn handle_key(&mut self, key: KeyEvent) -> VimAction {
        match self.mode {
            VimMode::Normal => self.handle_normal(key),
            VimMode::Insert => self.handle_insert(key),
            VimMode::Visual => self.handle_visual(key),
            VimMode::Command => self.handle_command(key),
        }
    }

    /// Return the full buffer content as a single string.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Current cursor position as `(row, col)`.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    /// Current mode.
    pub fn mode(&self) -> VimMode {
        self.mode
    }

    /// Human-readable mode indicator.
    pub fn mode_display(&self) -> &str {
        match self.mode {
            VimMode::Normal => "-- NORMAL --",
            VimMode::Insert => "-- INSERT --",
            VimMode::Visual => "-- VISUAL --",
            VimMode::Command => "-- COMMAND --",
        }
    }

    /// Contents of the `:` command buffer (only meaningful in Command mode).
    pub fn command_buffer(&self) -> &str {
        &self.command_buffer
    }

    // -- mode transitions ---------------------------------------------------

    fn enter_insert(&mut self, position: InsertPosition) {
        // Save undo before first insert session edit
        if !self.insert_undo_saved {
            self.save_undo();
            self.insert_undo_saved = true;
        }
        self.mode = VimMode::Insert;
        match position {
            InsertPosition::AtCursor => {}
            InsertPosition::AfterCursor => {
                let line_len = self.current_line_len();
                if self.cursor_col < line_len {
                    self.cursor_col += 1;
                }
            }
            InsertPosition::LineStart => {
                self.cursor_col = 0;
            }
            InsertPosition::LineEnd => {
                self.cursor_col = self.current_line_len();
            }
            InsertPosition::NewLineBelow => {
                let row = self.cursor_row;
                self.lines.insert(row + 1, String::new());
                self.cursor_row = row + 1;
                self.cursor_col = 0;
            }
            InsertPosition::NewLineAbove => {
                let row = self.cursor_row;
                self.lines.insert(row, String::new());
                // cursor_row stays the same (now points at the new empty line)
                self.cursor_col = 0;
            }
        }
    }

    fn enter_normal(&mut self) {
        self.mode = VimMode::Normal;
        self.insert_undo_saved = false;
        self.pending_operator = None;
        // Clamp cursor: in normal mode cursor should not be past last char
        let line_len = self.current_line_len();
        if line_len > 0 && self.cursor_col >= line_len {
            self.cursor_col = line_len - 1;
        }
    }

    fn enter_visual(&mut self) {
        self.mode = VimMode::Visual;
        self.visual_start = Some((self.cursor_row, self.cursor_col));
    }

    fn enter_command(&mut self) {
        self.mode = VimMode::Command;
        self.command_buffer.clear();
    }

    // -- mode handlers ------------------------------------------------------

    fn handle_normal(&mut self, key: KeyEvent) -> VimAction {
        // Enter in normal mode = submit the buffer (send message).
        if key.code == KeyCode::Enter && key.modifiers == KeyModifiers::NONE {
            return VimAction::Submit;
        }

        // Ctrl+R for redo — check before the main match to avoid
        // the plain 'r' arm swallowing it.
        if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.pending_count = None;
            self.redo();
            return VimAction::Redraw;
        }

        let count = self.take_count();

        // Handle pending operator (e.g. 'd' waiting for 'd', 'r' waiting for replacement char)
        if let Some(op) = self.pending_operator.take()
            && let KeyCode::Char(c) = key.code
        {
            match op {
                'r' => {
                    // 'r' + <char> replaces the character under cursor
                    self.save_undo();
                    self.redo_stack.clear();
                    self.replace_char(c);
                    return VimAction::Redraw;
                }
                'g' => {
                    if c == 'g' {
                        // gg — move to first line
                        self.cursor_row = 0;
                        self.clamp_col();
                        return VimAction::None;
                    }
                    // Unknown g-command — drop and fall through
                }
                _ if c == op => {
                    // dd, yy — line-wise operations
                    match op {
                        'd' => {
                            self.save_undo();
                            self.redo_stack.clear();
                            self.delete_line(count);
                            return VimAction::Redraw;
                        }
                        'y' => {
                            self.yank_line(count);
                            return VimAction::None;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // If the second key didn't match, drop the pending operator
        // and fall through to normal handling below.

        match key.code {
            // Count prefix digits (1-9 start, 0 only extends)
            KeyCode::Char(c @ '1'..='9') => {
                let digit = c as usize - '0' as usize;
                self.pending_count = Some(self.pending_count.unwrap_or(0) * 10 + digit);
                VimAction::None
            }
            KeyCode::Char('0') if self.pending_count.is_some() => {
                let current = self.pending_count.unwrap_or(0);
                self.pending_count = Some(current * 10);
                VimAction::None
            }

            // Movement
            KeyCode::Char('h') => {
                self.move_left(count);
                VimAction::None
            }
            KeyCode::Char('j') => {
                self.move_down(count);
                VimAction::None
            }
            KeyCode::Char('k') => {
                self.move_up(count);
                VimAction::None
            }
            KeyCode::Char('l') => {
                self.move_right(count);
                VimAction::None
            }
            KeyCode::Char('w') => {
                self.move_word_forward(count);
                VimAction::None
            }
            KeyCode::Char('b') => {
                self.move_word_backward(count);
                VimAction::None
            }
            KeyCode::Char('e') => {
                self.move_word_end(count);
                VimAction::None
            }
            KeyCode::Char('0') => {
                self.move_line_start();
                VimAction::None
            }
            KeyCode::Char('$') => {
                self.move_line_end();
                VimAction::None
            }

            // Insert mode entries
            KeyCode::Char('i') => {
                self.enter_insert(InsertPosition::AtCursor);
                VimAction::None
            }
            KeyCode::Char('a') => {
                self.enter_insert(InsertPosition::AfterCursor);
                VimAction::None
            }
            KeyCode::Char('I') => {
                self.enter_insert(InsertPosition::LineStart);
                VimAction::None
            }
            KeyCode::Char('A') => {
                self.enter_insert(InsertPosition::LineEnd);
                VimAction::None
            }
            KeyCode::Char('o') => {
                self.save_undo();
                self.insert_undo_saved = true;
                self.enter_insert(InsertPosition::NewLineBelow);
                VimAction::Redraw
            }
            KeyCode::Char('O') => {
                self.save_undo();
                self.insert_undo_saved = true;
                self.enter_insert(InsertPosition::NewLineAbove);
                VimAction::Redraw
            }

            // G — move to last line
            KeyCode::Char('G') => {
                self.cursor_row = self.lines.len().saturating_sub(1);
                self.clamp_col();
                VimAction::None
            }
            // g — pending operator for gg
            KeyCode::Char('g') => {
                self.pending_operator = Some('g');
                VimAction::None
            }

            // Visual mode
            KeyCode::Char('v') => {
                self.enter_visual();
                VimAction::None
            }

            // Command mode
            KeyCode::Char(':') => {
                self.enter_command();
                VimAction::None
            }

            // Operators that need a second key
            KeyCode::Char('d') => {
                self.pending_operator = Some('d');
                if count > 1 {
                    self.pending_count = Some(count);
                }
                VimAction::None
            }
            KeyCode::Char('y') => {
                self.pending_operator = Some('y');
                if count > 1 {
                    self.pending_count = Some(count);
                }
                VimAction::None
            }

            // Single-key editing
            KeyCode::Char('x') => {
                self.save_undo();
                self.redo_stack.clear();
                self.delete_char();
                VimAction::Redraw
            }
            KeyCode::Char('p') => {
                self.save_undo();
                self.redo_stack.clear();
                self.paste_after();
                VimAction::Redraw
            }
            KeyCode::Char('P') => {
                self.save_undo();
                self.redo_stack.clear();
                self.paste_before();
                VimAction::Redraw
            }
            KeyCode::Char('r') => {
                // 'r' needs a following character — set pending_operator.
                self.pending_operator = Some('r');
                VimAction::None
            }

            // Undo
            KeyCode::Char('u') => {
                self.undo();
                VimAction::Redraw
            }

            _ => VimAction::None,
        }
    }

    fn handle_insert(&mut self, key: KeyEvent) -> VimAction {
        match key.code {
            KeyCode::Esc => {
                self.enter_normal();
                VimAction::None
            }
            KeyCode::Char(c) => {
                self.insert_char(c);
                VimAction::Redraw
            }
            KeyCode::Backspace => {
                self.backspace();
                VimAction::Redraw
            }
            KeyCode::Enter => {
                self.insert_newline();
                VimAction::Redraw
            }
            _ => VimAction::None,
        }
    }

    fn handle_visual(&mut self, key: KeyEvent) -> VimAction {
        match key.code {
            KeyCode::Esc => {
                self.visual_start = None;
                self.enter_normal();
                VimAction::None
            }
            // Basic movement in visual mode
            KeyCode::Char('h') => {
                self.move_left(1);
                VimAction::None
            }
            KeyCode::Char('j') => {
                self.move_down(1);
                VimAction::None
            }
            KeyCode::Char('k') => {
                self.move_up(1);
                VimAction::None
            }
            KeyCode::Char('l') => {
                self.move_right(1);
                VimAction::None
            }
            KeyCode::Char('w') => {
                self.move_word_forward(1);
                VimAction::None
            }
            KeyCode::Char('b') => {
                self.move_word_backward(1);
                VimAction::None
            }
            KeyCode::Char('e') => {
                self.move_word_end(1);
                VimAction::None
            }
            KeyCode::Char('$') => {
                self.move_line_end();
                VimAction::None
            }
            KeyCode::Char('0') => {
                self.move_line_start();
                VimAction::None
            }
            // Yank selection
            KeyCode::Char('y') => {
                self.visual_yank();
                self.visual_start = None;
                self.enter_normal();
                VimAction::None
            }
            // Delete selection
            KeyCode::Char('d') | KeyCode::Char('x') => {
                self.save_undo();
                self.redo_stack.clear();
                self.visual_delete();
                self.visual_start = None;
                self.enter_normal();
                VimAction::Redraw
            }
            _ => VimAction::None,
        }
    }

    fn handle_command(&mut self, key: KeyEvent) -> VimAction {
        match key.code {
            KeyCode::Esc => {
                self.command_buffer.clear();
                self.enter_normal();
                VimAction::None
            }
            KeyCode::Enter => {
                let cmd = self.command_buffer.clone();
                self.command_buffer.clear();
                self.mode = VimMode::Normal;
                self.execute_command(&cmd)
            }
            KeyCode::Backspace => {
                self.command_buffer.pop();
                if self.command_buffer.is_empty() {
                    self.enter_normal();
                }
                VimAction::None
            }
            KeyCode::Char(c) => {
                self.command_buffer.push(c);
                VimAction::None
            }
            _ => VimAction::None,
        }
    }

    fn execute_command(&mut self, cmd: &str) -> VimAction {
        match cmd.trim() {
            "w" | "write" => VimAction::Submit,
            "q" | "quit" => VimAction::Quit,
            "wq" => VimAction::Submit, // submit implies quit in a TUI context
            _ => VimAction::None,
        }
    }

    // -- movement -----------------------------------------------------------

    fn move_left(&mut self, count: usize) {
        for _ in 0..count {
            if self.cursor_col > 0 {
                self.cursor_col -= 1;
            }
        }
    }

    fn move_right(&mut self, count: usize) {
        let max_col = self.max_cursor_col();
        for _ in 0..count {
            if self.cursor_col < max_col {
                self.cursor_col += 1;
            }
        }
    }

    fn move_up(&mut self, count: usize) {
        for _ in 0..count {
            if self.cursor_row > 0 {
                self.cursor_row -= 1;
            }
        }
        self.clamp_col();
    }

    fn move_down(&mut self, count: usize) {
        for _ in 0..count {
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
            }
        }
        self.clamp_col();
    }

    fn move_word_forward(&mut self, count: usize) {
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

    fn move_word_backward(&mut self, count: usize) {
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

    fn move_word_end(&mut self, count: usize) {
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

    fn move_line_start(&mut self) {
        self.cursor_col = 0;
    }

    fn move_line_end(&mut self) {
        let len = self.current_line_len();
        self.cursor_col = if len > 0 { len - 1 } else { 0 };
    }

    // -- editing ------------------------------------------------------------

    fn insert_char(&mut self, c: char) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        if row < self.lines.len() {
            self.lines[row].insert(col, c);
            self.cursor_col += c.len_utf8();
        }
    }

    fn backspace(&mut self) {
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

    fn insert_newline(&mut self) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        let rest = self.lines[row][col..].to_string();
        self.lines[row].truncate(col);
        self.lines.insert(row + 1, rest);
        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    fn delete_line(&mut self, count: usize) {
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

    fn yank_line(&mut self, count: usize) {
        let end = (self.cursor_row + count).min(self.lines.len());
        let yanked: Vec<&str> = self.lines[self.cursor_row..end]
            .iter()
            .map(|s| s.as_str())
            .collect();
        self.yank_buffer = yanked.join("\n");
        self.yank_is_linewise = true;
    }

    fn paste_after(&mut self) {
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

    fn paste_before(&mut self) {
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

    fn delete_char(&mut self) {
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

    fn replace_char(&mut self, c: char) {
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
    fn visual_yank(&mut self) {
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
    fn visual_delete(&mut self) {
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

    fn save_undo(&mut self) {
        self.undo_stack.push(UndoEntry {
            lines_before: self.lines.clone(),
            cursor_before: (self.cursor_row, self.cursor_col),
        });
    }

    fn undo(&mut self) {
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

    fn redo(&mut self) {
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

    // -- helpers ------------------------------------------------------------

    fn current_line_len(&self) -> usize {
        self.lines.get(self.cursor_row).map_or(0, |l| l.len())
    }

    /// Maximum column for cursor in current mode.
    fn max_cursor_col(&self) -> usize {
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

    fn clamp_col(&mut self) {
        let max = self.max_cursor_col();
        if self.cursor_col > max {
            self.cursor_col = max;
        }
    }

    /// Consume and return the pending count prefix (defaults to 1).
    fn take_count(&mut self) -> usize {
        self.pending_count.take().unwrap_or(1)
    }
}

impl Default for VimState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_normal_mode() {
        let s = VimState::default();
        assert_eq!(s.mode(), VimMode::Normal);
    }

    #[test]
    fn from_text_splits_lines() {
        let s = VimState::from_text("a\nb\nc");
        assert_eq!(s.text(), "a\nb\nc");
    }
}
