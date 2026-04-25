//! Vim-style keybinding state machine for the TUI input area.
//!
//! When `TuiConfig::vim_mode` is enabled, key events are routed through
//! [`VimState`] instead of the default [`InputHandler`](crate::input::InputHandler).
//!
//! Relocated from `src/vim.rs` → `src/vim/` per REM-2e (REM-2 split plan,
//! docs/rem-2-split-plan.md section 1). Zero public-API change: every
//! `archon_tui::vim::*` path is preserved via flat re-exports from this
//! file.
//!
//! Submodules (private to `vim`; public API is the impl block below):
//! - `handlers` — mode transitions (`enter_*`) + per-mode key dispatch
//!   (`handle_normal`/`handle_insert`/`handle_visual`/`handle_command`).
//! - `motions` — cursor movement primitives (`move_*`) + clamp helpers.
//! - `edits` — buffer editing (`insert_char`, `backspace`, `delete_*`,
//!   paste, visual selection) + undo/redo.
//!
//! Rust visibility rules mean child modules can access private items of
//! their parent (`UndoEntry`, `InsertPosition`, `VimState` fields).
//! Methods that cross sibling boundaries are marked `pub(super)` so
//! `handle_normal` (in `handlers`) can call `move_left` (in `motions`)
//! and `save_undo` (in `edits`).
//!
//! The 215-line `handle_normal` match in `handlers.rs` is kept intact —
//! its exhaustiveness is the correctness contract (see plan §1.4).
//!
//! Note: `TuiConfig` lives here so `archon-tui` stays self-contained;
//! `archon-core` re-exports or embeds it as needed.

use crossterm::event::KeyEvent;
use serde::{Deserialize, Serialize};

mod edits;
mod handlers;
mod motions;

// ---------------------------------------------------------------------------
// TuiConfig
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
// Internal helpers (private to `vim`; child submodules have access)
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
