use crate::commands::{self, CommandInfo};
use crate::ultrathink::UltrathinkState;

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
        self.suggestions.get(self.selected_index).map(|c| c.name)
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

            let matched: Vec<CommandInfo> = commands::filter_commands(prefix)
                .into_iter()
                .cloned()
                .collect();
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
            None => return,
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
    }

    /// Inject `text` at the current cursor position (voice input integration).
    pub fn inject_text(&mut self, text: &str) {
        self.current.insert_str(self.cursor_pos, text);
        self.cursor_pos += text.len();
        self.refresh_suggestions();
        self.ultrathink.scan_input(&self.current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_input() {
        let mut input = InputHandler::new();
        input.insert('H');
        input.insert('i');
        assert_eq!(input.text(), "Hi");
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn backspace() {
        let mut input = InputHandler::new();
        input.insert('a');
        input.insert('b');
        input.backspace();
        assert_eq!(input.text(), "a");
    }

    #[test]
    fn submit_clears_and_adds_history() {
        let mut input = InputHandler::new();
        input.insert('x');
        let text = input.submit();
        assert_eq!(text, "x");
        assert!(input.text().is_empty());
    }

    #[test]
    fn suggestions_activate_on_slash() {
        let mut input = InputHandler::new();
        input.insert('/');
        assert!(input.suggestions.active);
        assert!(!input.suggestions.suggestions.is_empty());
    }

    #[test]
    fn suggestions_deactivate_on_dismiss() {
        let mut input = InputHandler::new();
        input.insert('/');
        assert!(input.suggestions.active);
        input.dismiss_suggestions();
        assert!(!input.suggestions.active);
    }

    #[test]
    fn tab_completes_selected_command() {
        let mut input = InputHandler::new();
        // Type "/mo" to filter to /model
        for ch in "/mo".chars() {
            input.insert(ch);
        }
        assert!(input.suggestions.active);
        assert_eq!(input.suggestions.suggestions.len(), 1);
        let accepted = input.accept_suggestion();
        assert!(accepted);
        assert!(input.text().starts_with("/model"));
        assert!(!input.suggestions.active);
    }

    #[test]
    fn suggestions_deactivate_on_non_slash() {
        let mut input = InputHandler::new();
        input.insert('h');
        assert!(!input.suggestions.active);
    }

    #[test]
    fn suggestions_deactivate_on_backspace_past_slash() {
        let mut input = InputHandler::new();
        input.insert('/');
        assert!(input.suggestions.active);
        input.backspace();
        assert!(!input.suggestions.active);
    }

    #[test]
    fn suggestions_dismiss_when_argument_typed() {
        let mut input = InputHandler::new();
        // Type "/model" — suggestions active
        for ch in "/model".chars() {
            input.insert(ch);
        }
        assert!(input.suggestions.active);
        // Type space + "haiku" — suggestions should dismiss
        input.insert(' ');
        assert!(
            !input.suggestions.active,
            "suggestions stayed active after argument typed"
        );
        for ch in "haiku".chars() {
            input.insert(ch);
        }
        assert!(!input.suggestions.active);
        assert_eq!(input.text(), "/model haiku");
    }

    #[test]
    fn suggestions_stay_active_for_partial_prefix() {
        let mut input = InputHandler::new();
        for ch in "/mo".chars() {
            input.insert(ch);
        }
        assert!(input.suggestions.active);
        // No space yet — still completing
        assert!(
            input
                .suggestions
                .suggestions
                .iter()
                .any(|c| c.name == "/model")
        );
    }

    #[test]
    fn history_navigation() {
        let mut input = InputHandler::new();
        input.insert('a');
        input.submit();
        input.insert('b');
        input.submit();

        input.history_up();
        assert_eq!(input.text(), "b");
        input.history_up();
        assert_eq!(input.text(), "a");
        input.history_down();
        assert_eq!(input.text(), "b");
        input.history_down();
        assert!(input.text().is_empty());
    }
}
