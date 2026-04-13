//! Session browser view (TASK-AGS-616 stub scaffold).
//!
//! This is the initial scaffolding for the session browser view. The full
//! migration of session tree rendering, `TuiEvent::ResumeSession` event
//! emission, and integration with the `archon-session` crate is
//! intentionally deferred to a later phase — see
//! `SESSION_BROWSER_PLACEHOLDER`.
//!
//! For now this module exposes the minimal API surface
//! (`SessionId`, `SessionNode`, `SessionBrowserState`, `open`, `draw`,
//! `on_key`, `on_agent_event`) so subsequent work and tests can depend on
//! it. The `pending_resume` field on `SessionBrowserState` is the
//! placeholder for the eventual `TuiEvent::ResumeSession(SessionId)`
//! emission: when `Enter` is pressed the selected session id is stored
//! into `pending_resume` instead of being dispatched as an event. The
//! event-bus migration will replace this field once `app.rs` is touched
//! in a later phase.
//!
//! Per the per-view isolation rule, this module MUST NOT import from any
//! other `crate::views::*` module. It also MUST NOT import from the
//! `archon_session` crate yet — `SessionId` is intentionally a local
//! placeholder type alias that will be re-pointed at the real type during
//! the full migration.

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

/// Placeholder for the full session browser view content.
///
/// The real session tree rendering, event emission, and `archon-session`
/// integration are deferred to a later task. This constant exists so
/// future modules and tests have a single symbol to point at when wiring
/// session browser content.
pub const SESSION_BROWSER_PLACEHOLDER: &str =
    "Session browser view (placeholder — full migration with TuiEvent::ResumeSession + archon-session integration in later phase)";

/// Local placeholder for a session identifier.
///
/// Intentionally a type alias so it can be re-pointed at the real
/// `archon_session::SessionId` (or equivalent) during the full migration
/// without touching call sites.
pub type SessionId = String;

/// Local placeholder for a node in the session tree.
///
/// The real session tree model lives in `archon-session` and will replace
/// this type during the full migration. For now this carries only the
/// minimum needed to drive cursor movement and `pending_resume` in the
/// stub.
#[derive(Debug, Default, Clone)]
pub struct SessionNode {
    /// Stable identifier for the session represented by this node.
    pub id: SessionId,
    /// Human-readable label for the node.
    pub label: String,
    /// Child sessions (e.g. forks/branches). Unused by the stub renderer
    /// but kept so the shape matches the eventual real type.
    pub children: Vec<SessionNode>,
}

/// State for the session browser view.
///
/// Currently tracks the flat list of sessions, cursor position, the set
/// of expanded session ids, and the pending resume target. The
/// `pending_resume` field is the stub placeholder for emitting
/// `TuiEvent::ResumeSession(SessionId)` once the event bus migration is
/// allowed (see module docs).
#[derive(Debug, Default, Clone)]
pub struct SessionBrowserState {
    /// Session entries to render. Each entry is a `SessionNode` for now;
    /// the richer tree rendering will arrive with the full migration.
    pub sessions: Vec<SessionNode>,
    /// Index of the currently highlighted session. Always clamped to
    /// `0..sessions.len()` (or `0` when `sessions` is empty).
    pub cursor: usize,
    /// Set of session ids whose children are currently expanded in the
    /// rendered tree. Unused by the stub renderer but kept so future
    /// expansion/collapse logic has a stable home.
    pub expanded: HashSet<SessionId>,
    /// Selected session id awaiting resume dispatch. Set when the user
    /// presses `Enter` on a session and cleared on `Esc` or `open`. This
    /// is the stub stand-in for `TuiEvent::ResumeSession(SessionId)`.
    pub pending_resume: Option<SessionId>,
}

/// Open (or re-open) the session browser view.
///
/// Stub behaviour: clears any pending resume target and resets the
/// cursor to the top of the list. The eventual implementation will also
/// kick off a refresh of the session list from `archon-session`.
pub fn open(state: &mut SessionBrowserState) {
    state.pending_resume = None;
    state.cursor = 0;
}

/// Draw the session browser view into `area`.
///
/// Renders an empty bordered block titled "Session Browser" — actual
/// tree rendering is deferred to the full migration.
pub fn draw(frame: &mut Frame, area: Rect, _state: &SessionBrowserState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Session Browser");
    frame.render_widget(block, area);
}

/// Handle a key event for the session browser view.
///
/// Returns `false` because this scaffold does not yet consume input via
/// the parent app event loop in any meaningful way. The cursor is
/// advanced/rewound on `Down`/`j` and `Up`/`k`, `Enter` records the
/// selected session id into `pending_resume` (the placeholder for
/// `TuiEvent::ResumeSession`), and `Esc` clears any pending resume.
pub fn on_key(state: &mut SessionBrowserState, key_code: KeyCode) -> bool {
    match key_code {
        KeyCode::Down | KeyCode::Char('j') => {
            let max_index = state.sessions.len().saturating_sub(1);
            if state.cursor < max_index {
                state.cursor += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Enter => {
            if let Some(node) = state.sessions.get(state.cursor) {
                state.pending_resume = Some(node.id.clone());
            }
        }
        KeyCode::Esc => {
            state.pending_resume = None;
        }
        _ => {}
    }
    false
}

/// React to an agent event.
///
/// No-op stub. The full implementation will fold session lifecycle
/// updates (created, resumed, closed) into the rendered tree.
pub fn on_agent_event(_state: &mut SessionBrowserState) {}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    fn node(id: &str) -> SessionNode {
        SessionNode {
            id: id.to_string(),
            label: id.to_string(),
            children: Vec::new(),
        }
    }

    #[test]
    fn session_browser_state_default_empty() {
        let state = SessionBrowserState::default();
        assert!(state.sessions.is_empty(), "default sessions should be empty");
        assert_eq!(state.cursor, 0, "default cursor should be 0");
        assert!(state.expanded.is_empty(), "default expanded set should be empty");
        assert!(
            state.pending_resume.is_none(),
            "default pending_resume should be None"
        );
    }

    #[test]
    fn open_resets_state() {
        let mut state = SessionBrowserState {
            sessions: vec![node("a"), node("b")],
            cursor: 5,
            expanded: HashSet::new(),
            pending_resume: Some("foo".to_string()),
        };
        open(&mut state);
        assert_eq!(state.cursor, 0, "open should reset cursor to 0");
        assert!(
            state.pending_resume.is_none(),
            "open should clear pending_resume"
        );
    }

    #[test]
    fn on_key_down_advances_cursor() {
        let mut state = SessionBrowserState {
            sessions: vec![node("alpha"), node("bravo")],
            cursor: 0,
            ..Default::default()
        };
        let consumed = on_key(&mut state, KeyCode::Down);
        assert!(!consumed, "stub on_key should not consume input");
        assert_eq!(state.cursor, 1, "Down should advance cursor to 1");
    }

    #[test]
    fn on_key_down_clamps_at_end() {
        let mut state = SessionBrowserState {
            sessions: vec![node("alpha")],
            cursor: 0,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Down);
        assert_eq!(
            state.cursor, 0,
            "Down on single-entry list should keep cursor clamped at 0"
        );
    }

    #[test]
    fn on_key_up_saturates_at_zero() {
        let mut state = SessionBrowserState {
            sessions: vec![node("alpha")],
            cursor: 0,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Up);
        assert_eq!(state.cursor, 0, "Up at cursor 0 should saturate at 0");
    }

    #[test]
    fn on_key_enter_sets_pending_resume() {
        let mut state = SessionBrowserState {
            sessions: vec![node("abc")],
            cursor: 0,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Enter);
        assert_eq!(
            state.pending_resume,
            Some("abc".to_string()),
            "Enter should record selected session id into pending_resume"
        );
    }

    #[test]
    fn on_key_esc_clears_pending_resume() {
        let mut state = SessionBrowserState {
            sessions: vec![node("xyz")],
            cursor: 0,
            pending_resume: Some("xyz".to_string()),
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Esc);
        assert!(
            state.pending_resume.is_none(),
            "Esc should clear pending_resume"
        );
    }

    #[test]
    fn on_key_j_advances_cursor() {
        let mut state = SessionBrowserState {
            sessions: vec![node("alpha"), node("bravo")],
            cursor: 0,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Char('j'));
        assert_eq!(state.cursor, 1, "j should advance cursor like Down");
    }

    #[test]
    fn on_key_k_decrements_cursor() {
        let mut state = SessionBrowserState {
            sessions: vec![node("alpha"), node("bravo")],
            cursor: 1,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Char('k'));
        assert_eq!(state.cursor, 0, "k should decrement cursor like Up");
    }

    #[test]
    fn draw_does_not_panic() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = SessionBrowserState::default();
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw should not panic on default state");
    }
}
