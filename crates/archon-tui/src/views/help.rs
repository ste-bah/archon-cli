//! Help overlay view (TASK-AGS-609 stub scaffold).
//!
//! This is the initial scaffolding for the help overlay extracted from
//! `app.rs`. The full migration of help text and key bindings is intentionally
//! deferred to a later phase — see `HELP_TEXT_PLACEHOLDER`. For now this
//! module exposes the minimal API surface (`HelpOverlayState` and
//! `draw_help`) so subsequent view modules and tests can depend on it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

/// Placeholder for the full help text + key bindings table.
///
/// The real binding table lives in `app.rs` for now and will be migrated
/// out as a follow-up task. This constant exists so future view modules
/// (and tests) have a single symbol to point at when wiring help content.
pub const HELP_TEXT_PLACEHOLDER: &str =
    "Help bindings (placeholder — full migration in later phase)";

/// State for the help overlay view.
///
/// Currently only tracks visibility. Future fields may include scroll
/// offset, search query, or a selected category.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HelpOverlayState {
    /// Whether the help overlay should be drawn.
    pub visible: bool,
}

/// Draw the help overlay into `area`.
///
/// When `state.visible` is `false` this is a no-op so callers can
/// unconditionally invoke it during their render pass. When visible, it
/// renders an empty bordered block titled "Help" — actual content rendering
/// is deferred to the full migration of help text out of `app.rs`.
pub fn draw_help(frame: &mut Frame, area: Rect, state: &HelpOverlayState) {
    if !state.visible {
        return;
    }
    let block = Block::default().borders(Borders::ALL).title("Help");
    frame.render_widget(block, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn help_overlay_state_default_is_hidden() {
        let state = HelpOverlayState::default();
        assert!(!state.visible, "default HelpOverlayState should be hidden");
    }

    #[test]
    fn help_overlay_state_can_be_made_visible() {
        let mut state = HelpOverlayState::default();
        state.visible = true;
        assert!(state.visible, "HelpOverlayState.visible should toggle to true");
    }

    #[test]
    fn draw_help_does_not_panic_when_hidden() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = HelpOverlayState { visible: false };
        terminal
            .draw(|f| draw_help(f, f.area(), &state))
            .expect("draw_help (hidden) should not panic");
    }

    #[test]
    fn draw_help_does_not_panic_when_visible() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = HelpOverlayState { visible: true };
        terminal
            .draw(|f| draw_help(f, f.area(), &state))
            .expect("draw_help (visible) should not panic");
    }
}
