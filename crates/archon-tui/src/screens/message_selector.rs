//! TASK-TUI-620 message-selector overlay (screen module).
//!
//! Scrollable list of prior conversation messages. User navigates via
//! Up/Down arrows; Enter selects and closes. Esc cancels.
//!
//! Gate 2 scope: struct + selection-navigation methods + unit tests.
//! Render + full input routing + truncation-on-confirm are deferred
//! to a TUI-620-followup ticket per the orchestrator scope-reduction.

use crate::events::MessageSummary;

pub struct MessageSelector {
    pub messages: Vec<MessageSummary>,
    pub selected_index: usize,
}

impl MessageSelector {
    pub fn new(messages: Vec<MessageSummary>) -> Self {
        Self {
            messages,
            selected_index: 0,
        }
    }

    /// Move selection down by one (wraps to 0 at bottom).
    pub fn select_next(&mut self) {
        if self.messages.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.messages.len();
    }

    /// Move selection up by one (wraps to last at top).
    pub fn select_prev(&mut self) {
        if self.messages.is_empty() {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            self.messages.len() - 1
        } else {
            self.selected_index - 1
        };
    }

    pub fn selected(&self) -> Option<&MessageSummary> {
        self.messages.get(self.selected_index)
    }

    // TODO(TUI-620-followup): render(frame, area) — draw numbered
    // list, highlight self.selected_index with reverse video. See
    // `screens/session_browser.rs` render() for the canonical pattern.
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn fixture(n: usize) -> Vec<MessageSummary> {
        (0..n)
            .map(|i| MessageSummary {
                id: format!("msg-{}", i),
                timestamp: Utc::now(),
                preview: format!("preview-{}", i),
            })
            .collect()
    }

    #[test]
    fn new_starts_at_zero() {
        let sel = MessageSelector::new(fixture(3));
        assert_eq!(sel.selected_index, 0);
    }

    #[test]
    fn select_next_advances() {
        let mut sel = MessageSelector::new(fixture(3));
        sel.select_next();
        assert_eq!(sel.selected_index, 1);
        sel.select_next();
        assert_eq!(sel.selected_index, 2);
    }

    #[test]
    fn select_next_wraps_at_end() {
        let mut sel = MessageSelector::new(fixture(3));
        sel.selected_index = 2;
        sel.select_next();
        assert_eq!(sel.selected_index, 0);
    }

    #[test]
    fn select_prev_wraps_at_start() {
        let mut sel = MessageSelector::new(fixture(3));
        sel.select_prev();
        assert_eq!(sel.selected_index, 2);
    }

    #[test]
    fn empty_list_noop() {
        let mut sel = MessageSelector::new(vec![]);
        sel.select_next();
        sel.select_prev();
        assert_eq!(sel.selected_index, 0);
        assert!(sel.selected().is_none());
    }
}
