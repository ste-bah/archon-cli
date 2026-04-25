//! Mode transitions and per-mode key dispatch for `VimState`.
//!
//! Relocated from `src/vim.rs` (mode transitions L169-L224 + handle_normal
//! L228-L441 + handle_insert L443-L463 + handle_visual L465-L527 +
//! handle_command L529-L555 + execute_command L557-L564) per REM-2e.
//!
//! `handle_normal` is kept as a single 200+ line match — its
//! exhaustiveness is the correctness contract. Do NOT fragment.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{InsertPosition, VimAction, VimMode, VimState};

impl VimState {
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

    #[allow(clippy::too_many_lines)]
    pub(super) fn handle_normal(&mut self, key: KeyEvent) -> VimAction {
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

    pub(super) fn handle_insert(&mut self, key: KeyEvent) -> VimAction {
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

    pub(super) fn handle_visual(&mut self, key: KeyEvent) -> VimAction {
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

    pub(super) fn handle_command(&mut self, key: KeyEvent) -> VimAction {
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
}
