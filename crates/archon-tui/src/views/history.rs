//! History view (TASK-AGS-613 stub scaffold).
//!
//! This is the initial scaffolding for the history view. The full migration
//! of history rendering and event handling from `app.rs` is intentionally
//! deferred to a later phase — see `HISTORY_PLACEHOLDER`. For now this
//! module exposes the minimal API surface (`HistoryViewState`, `draw`,
//! `on_key`, `on_agent_event`) so subsequent work and tests can depend on it.
//!
//! Per the per-view isolation rule, this module MUST NOT import from any
//! other `crate::views::*` module.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

/// Placeholder for the full history rendering content.
///
/// The real history list and detail rendering currently live in `app.rs`
/// and will be migrated out as a follow-up task. This constant exists so
/// future modules and tests have a single symbol to point at when wiring
/// history content.
pub const HISTORY_PLACEHOLDER: &str =
    "History view (placeholder — full migration in later phase)";

/// State for the history view.
///
/// Currently only tracks the list of entries and the cursor position.
/// Future fields may include scroll offset, filter query, or selection
/// metadata.
#[derive(Debug, Default, Clone)]
pub struct HistoryViewState {
    /// History entries to render. Each entry is a single rendered line
    /// for now; a richer structured type will replace this in the full
    /// migration.
    pub entries: Vec<String>,
    /// Index of the currently highlighted entry. Always clamped to
    /// `0..entries.len()` (or `0` when `entries` is empty).
    pub cursor: usize,
}

/// Draw the history view into `area`.
///
/// Renders an empty bordered block titled "History" — actual content
/// rendering is deferred to the full migration of history handling out
/// of `app.rs`.
pub fn draw(frame: &mut Frame, area: Rect, _state: &HistoryViewState) {
    let block = Block::default().borders(Borders::ALL).title("History");
    frame.render_widget(block, area);
}

/// Handle a key event for the history view.
///
/// Returns `false` because this scaffold does not yet consume input in
/// any meaningful way. The cursor is still advanced or rewound on
/// `Up`/`Down` so future tests and wiring can rely on that minimal
/// behaviour.
pub fn on_key(state: &mut HistoryViewState, key_code: KeyCode) -> bool {
    match key_code {
        KeyCode::Up => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Down => {
            let max_index = state.entries.len().saturating_sub(1);
            if state.cursor < max_index {
                state.cursor += 1;
            }
        }
        _ => {}
    }
    false
}

/// React to an agent event.
///
/// No-op stub. The full implementation will append new history entries
/// based on agent event payloads.
pub fn on_agent_event(_state: &mut HistoryViewState) {}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    #[test]
    fn history_view_state_default_is_empty() {
        let state = HistoryViewState::default();
        assert!(state.entries.is_empty(), "default entries should be empty");
        assert_eq!(state.cursor, 0, "default cursor should be 0");
    }

    #[test]
    fn on_key_down_advances_cursor() {
        let mut state = HistoryViewState {
            entries: vec!["a".to_string(), "b".to_string()],
            cursor: 0,
        };
        let consumed = on_key(&mut state, KeyCode::Down);
        assert!(!consumed, "stub on_key should not consume input");
        assert_eq!(state.cursor, 1, "Down should advance cursor to 1");
    }

    #[test]
    fn on_key_down_clamps_at_end() {
        let mut state = HistoryViewState {
            entries: vec!["a".to_string()],
            cursor: 0,
        };
        on_key(&mut state, KeyCode::Down);
        assert_eq!(
            state.cursor, 0,
            "Down on single-entry list should keep cursor clamped at 0"
        );
    }

    #[test]
    fn on_key_up_decrements_cursor() {
        let mut state = HistoryViewState {
            entries: vec!["a".to_string(), "b".to_string()],
            cursor: 1,
        };
        on_key(&mut state, KeyCode::Up);
        assert_eq!(state.cursor, 0, "Up should decrement cursor to 0");
    }

    #[test]
    fn on_key_up_saturates_at_zero() {
        let mut state = HistoryViewState {
            entries: vec!["a".to_string()],
            cursor: 0,
        };
        on_key(&mut state, KeyCode::Up);
        assert_eq!(state.cursor, 0, "Up at cursor 0 should saturate at 0");
    }

    #[test]
    fn draw_does_not_panic() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = HistoryViewState::default();
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw should not panic on default state");
    }
}
