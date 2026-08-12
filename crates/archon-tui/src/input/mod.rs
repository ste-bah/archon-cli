//! Input subsystem for the TUI.
//!
//! This module owns the text-editing buffer ([`InputHandler`]) with cursor
//! management, slash-command autocomplete state ([`SuggestionState`]),
//! history navigation, voice-injection entry points, and ultrathink keyword
//! scanning. App-level key dispatch — translating [`crate::keybindings::Action`]
//! values into calls on [`InputHandler`] and returning a [`KeyResult`] to the
//! event loop — lives in the [`dispatch`] submodule and is re-exported here
//! so callers keep using `crate::input::handle_key` and `crate::input::KeyResult`.
//!
//! Split from a single 527-line file (TASK-HYGIENE-INPUT-RS-SPLIT) to clear
//! the 500-line FileSizeGuard invariant; zero behavioral change.

use crate::commands::{self, CommandInfo};
use crate::ultrathink::UltrathinkState;

pub mod dispatch;
pub use dispatch::{KeyResult, handle_key};

/// Tracks the state of the slash-command autocomplete popup.
#[derive(Debug, Default)]
pub struct SuggestionState {
    /// Whether the suggestion popup is currently visible.
    pub active: bool,
    /// Filtered list of matching commands.
    pub suggestions: Vec<CommandInfo>,
    /// Index of the currently highlighted suggestion.
    pub selected_index: usize,
}

impl SuggestionState {
    /// Dismiss the popup, clearing all state.
    pub fn deactivate(&mut self) {
        self.active = false;
        self.suggestions.clear();
        self.selected_index = 0;
    }

    /// Move selection up by one, wrapping around.
    pub fn select_prev(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.suggestions.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    /// Move selection down by one, wrapping around.
    pub fn select_next(&mut self) {
        if self.suggestions.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.suggestions.len();
    }

    /// Return the currently selected command name, if any.
    pub fn selected_name(&self) -> Option<&str> {
        self.suggestions
            .get(self.selected_index)
            .map(|c| c.name.as_str())
    }
}

/// Input handler with history, multi-line support, slash-command suggestions,
/// and ultrathink keyword detection.
#[derive(Debug, Default)]
pub struct InputHandler {
    current: String,
    cursor_pos: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    pub suggestions: SuggestionState,
    pub ultrathink: UltrathinkState,
}

impl InputHandler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the current input text.
    pub fn text(&self) -> &str {
        &self.current
    }

    /// Get cursor position.
    pub fn cursor(&self) -> usize {
        self.cursor_pos
    }

    /// Insert a character at cursor position.
    pub fn insert(&mut self, ch: char) {
        self.current.insert(self.cursor_pos, ch);
        self.cursor_pos += ch.len_utf8();
        self.refresh_suggestions();
        self.ultrathink.scan_input(&self.current);
        self.trace("insert");
    }

    /// Insert a literal newline at the cursor (Shift+Enter / Alt+Enter).
    ///
    /// A separate entry point from [`InputHandler::insert`] so the
    /// `ARCHON_TUI_LOG_KEYS` trace distinguishes "the user asked for a
    /// newline" from "a newline arrived as text" (paste, voice injection) —
    /// the two look identical in the buffer and have completely different
    /// causes when a draft ends up multi-line unexpectedly.
    pub fn insert_newline(&mut self) {
        self.current.insert(self.cursor_pos, '\n');
        self.cursor_pos += 1;
        self.refresh_suggestions();
        self.ultrathink.scan_input(&self.current);
        self.trace("insert_newline");
    }

    /// Emit one buffer-state line to the `ARCHON_TUI_LOG_KEYS` trace.
    ///
    /// Returns immediately when the trace is off, which is every production
    /// run — see [`crate::keylog`].
    fn trace(&self, op: &str) {
        crate::keylog::log_buffer(op, &self.current, self.cursor_pos);
    }

    /// Delete character before cursor (backspace).
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.current[..self.cursor_pos]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_pos -= prev;
            self.current.remove(self.cursor_pos);
        }
        self.refresh_suggestions();
        self.ultrathink.scan_input(&self.current);
        self.trace("backspace");
    }

    /// Move cursor left.
    pub fn move_left(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.current[..self.cursor_pos]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_pos -= prev;
        }
    }

    /// Move cursor right.
    pub fn move_right(&mut self) {
        if self.cursor_pos < self.current.len() {
            let next = self.current[self.cursor_pos..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_pos += next;
        }
    }

    /// Replace the current input text and place the cursor at the end.
    /// Used by overlays (e.g. `/skills` Enter-to-inject, TUI-627-followup)
    /// to write a command template directly into the input buffer.
    pub fn set_text(&mut self, text: &str) {
        self.current = text.to_string();
        self.cursor_pos = self.current.len();
        self.refresh_suggestions();
        self.ultrathink.scan_input(&self.current);
        self.trace("set_text");
    }

    /// Update suggestion state based on current input text.
    fn refresh_suggestions(&mut self) {
        if self.current.starts_with('/') {
            let prefix = self
                .current
                .split_whitespace()
                .next()
                .unwrap_or(&self.current);
            // If there's a space after the command name, the user is typing
            // an argument — dismiss suggestions. split_whitespace ignores
            // trailing spaces, so check for space after the prefix directly.
            let has_argument = self.current.len() > prefix.len();

            // If the user has already typed an argument after the command name,
            // dismiss suggestions — they're past the completion phase.
            if has_argument {
                self.suggestions.deactivate();
                return;
            }

            let matched: Vec<CommandInfo> = commands::filter_commands(prefix);
            if matched.is_empty() {
                self.suggestions.deactivate();
            } else {
                self.suggestions.active = true;
                self.suggestions.suggestions = matched;
                if self.suggestions.selected_index >= self.suggestions.suggestions.len() {
                    self.suggestions.selected_index = 0;
                }
            }
        } else {
            self.suggestions.deactivate();
        }
    }

    /// Accept the currently selected suggestion, replacing input with the command name.
    /// Returns `true` if a suggestion was accepted.
    pub fn accept_suggestion(&mut self) -> bool {
        if !self.suggestions.active {
            return false;
        }
        if let Some(name) = self.suggestions.selected_name() {
            let name = name.to_string();
            self.current = format!("{name} ");
            self.cursor_pos = self.current.len();
            self.suggestions.deactivate();
            self.ultrathink.scan_input(&self.current);
            return true;
        }
        false
    }

    /// Dismiss suggestions without accepting.
    pub fn dismiss_suggestions(&mut self) {
        self.suggestions.deactivate();
    }

    /// Submit the current input, add to history, return the text.
    pub fn submit(&mut self) -> String {
        self.suggestions.deactivate();
        let text = std::mem::take(&mut self.current);
        self.cursor_pos = 0;
        self.history_index = None;
        self.ultrathink.scan_input("");

        if !text.trim().is_empty() {
            self.history.push(text.clone());
        }

        crate::keylog::log_buffer("submit", &text, 0);
        text
    }

    /// Navigate history up (older).
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }

        let idx = match self.history_index {
            None => self.history.len() - 1,
            Some(i) if i > 0 => i - 1,
            Some(_) => return,
        };

        self.history_index = Some(idx);
        self.current = self.history[idx].clone();
        self.cursor_pos = self.current.len();
        self.ultrathink.scan_input(&self.current);
    }

    /// Navigate history down (newer).
    pub fn history_down(&mut self) {
        match self.history_index {
            None => (),
            Some(i) => {
                if i + 1 < self.history.len() {
                    self.history_index = Some(i + 1);
                    self.current = self.history[i + 1].clone();
                } else {
                    self.history_index = None;
                    self.current.clear();
                }
                self.cursor_pos = self.current.len();
                self.ultrathink.scan_input(&self.current);
            }
        }
    }

    /// Clear the input.
    pub fn clear(&mut self) {
        self.current.clear();
        self.cursor_pos = 0;
        self.history_index = None;
        self.ultrathink.scan_input(&self.current);
        self.trace("clear");
    }

    /// Inject `text` at the current cursor position (voice input integration).
    pub fn inject_text(&mut self, text: &str) {
        self.current.insert_str(self.cursor_pos, text);
        self.cursor_pos += text.len();
        self.refresh_suggestions();
        self.ultrathink.scan_input(&self.current);
        self.trace("inject_text");
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
