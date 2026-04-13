//! Settings view (TASK-AGS-615 stub scaffold).
//!
//! This is the initial scaffolding for the settings view. The full migration
//! of settings rendering and event handling from `app.rs` is intentionally
//! deferred to a later phase — see `SETTINGS_PLACEHOLDER`. For now this
//! module exposes the minimal API surface (`SettingsViewState`, `draw`,
//! `on_key`, `on_agent_event`) so subsequent work and tests can depend on it.
//!
//! Per the per-view isolation rule, this module MUST NOT import from any
//! other `crate::views::*` module.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

/// Placeholder for the full settings view content.
///
/// The real settings rendering currently lives in `app.rs` and will be
/// migrated out as a follow-up task. This constant exists so future
/// modules and tests have a single symbol to point at when wiring
/// settings content.
pub const SETTINGS_PLACEHOLDER: &str =
    "Settings view (placeholder — full migration in later phase)";

/// State for the settings view.
///
/// Currently tracks a list of `(key, value)` entries, the cursor position,
/// and a buffer for in-progress edits. Future fields may include a
/// validation status, dirty flag, or richer per-entry metadata.
#[derive(Debug, Default, Clone)]
pub struct SettingsViewState {
    /// Settings entries to render. Each entry is a `(key, value)` pair;
    /// a richer structured type will replace this in the full migration.
    pub entries: Vec<(String, String)>,
    /// Index of the currently highlighted entry. Always clamped to
    /// `0..entries.len()` (or `0` when `entries` is empty).
    pub cursor: usize,
    /// Buffer holding the in-progress edit value for the current entry.
    /// Empty when no edit is in progress.
    pub edit_buffer: String,
}

/// Draw the settings view into `area`.
///
/// Renders an empty bordered block titled "Settings" — actual content
/// rendering is deferred to the full migration of settings handling out
/// of `app.rs`.
pub fn draw(frame: &mut Frame, area: Rect, _state: &SettingsViewState) {
    let block = Block::default().borders(Borders::ALL).title("Settings");
    frame.render_widget(block, area);
}

/// Handle a key event for the settings view.
///
/// Returns `false` because this scaffold does not yet consume input in
/// any meaningful way. The cursor is still advanced or rewound on
/// `Up`/`Down` so future tests and wiring can rely on that minimal
/// behaviour.
pub fn on_key(state: &mut SettingsViewState, key_code: KeyCode) -> bool {
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
/// No-op stub. The full implementation will refresh settings entries
/// based on agent event payloads (e.g. config reload notifications).
pub fn on_agent_event(_state: &mut SettingsViewState) {}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    #[test]
    fn settings_view_state_default_is_empty() {
        let state = SettingsViewState::default();
        assert!(state.entries.is_empty(), "default entries should be empty");
        assert_eq!(state.cursor, 0, "default cursor should be 0");
        assert!(
            state.edit_buffer.is_empty(),
            "default edit_buffer should be empty"
        );
    }

    #[test]
    fn on_key_down_advances_cursor() {
        let mut state = SettingsViewState {
            entries: vec![
                ("theme".to_string(), "dark".to_string()),
                ("model".to_string(), "opus".to_string()),
            ],
            cursor: 0,
            edit_buffer: String::new(),
        };
        let consumed = on_key(&mut state, KeyCode::Down);
        assert!(!consumed, "stub on_key should not consume input");
        assert_eq!(state.cursor, 1, "Down should advance cursor to 1");
    }

    #[test]
    fn on_key_down_clamps_at_end() {
        let mut state = SettingsViewState {
            entries: vec![("theme".to_string(), "dark".to_string())],
            cursor: 0,
            edit_buffer: String::new(),
        };
        on_key(&mut state, KeyCode::Down);
        assert_eq!(
            state.cursor, 0,
            "Down on single-entry list should keep cursor clamped at 0"
        );
    }

    #[test]
    fn on_key_up_decrements_cursor() {
        let mut state = SettingsViewState {
            entries: vec![
                ("theme".to_string(), "dark".to_string()),
                ("model".to_string(), "opus".to_string()),
            ],
            cursor: 1,
            edit_buffer: String::new(),
        };
        on_key(&mut state, KeyCode::Up);
        assert_eq!(state.cursor, 0, "Up should decrement cursor to 0");
    }

    #[test]
    fn on_key_up_saturates_at_zero() {
        let mut state = SettingsViewState {
            entries: vec![("theme".to_string(), "dark".to_string())],
            cursor: 0,
            edit_buffer: String::new(),
        };
        on_key(&mut state, KeyCode::Up);
        assert_eq!(state.cursor, 0, "Up at cursor 0 should saturate at 0");
    }

    #[test]
    fn draw_does_not_panic() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = SettingsViewState::default();
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw should not panic on default state");
    }
}
