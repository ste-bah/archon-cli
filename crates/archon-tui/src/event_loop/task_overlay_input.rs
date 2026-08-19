//! Key routing for the tasks overlay (#189 Phase 9).
//!
//! Split out of `event_loop/input.rs` to keep that file under the 500-line
//! ceiling the `preserve_file_size_ceiling_gate` test enforces.

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;

/// Route one key while the tasks overlay is open.
///
/// Returns `true` when the key belonged to the overlay, which includes keys it
/// deliberately swallows — an overlay that let arbitrary keystrokes fall
/// through to the prompt would type into the input buffer behind itself.
///
/// Cancellation is a keypress here rather than an injected slash command the
/// user still has to submit: the point of the phase is that work visible in
/// this list can be stopped from it.
pub(crate) fn handle_task_overlay_key(app: &mut App, key: KeyEvent) -> bool {
    if app.task_overlay.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut overlay) = app.task_overlay {
                overlay.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut overlay) = app.task_overlay {
                overlay.move_down();
            }
        }
        KeyCode::Char('x') | KeyCode::Delete => {
            app.cancel_selected_task();
        }
        KeyCode::Char('r') => app.refresh_task_overlay(),
        KeyCode::Esc => app.close_task_overlay(),
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn keys_pass_through_when_the_overlay_is_closed() {
        let mut app = App::default();
        assert!(!handle_task_overlay_key(&mut app, key(KeyCode::Char('x'))));
    }

    /// Every key is consumed while the overlay is up, or typing lands in the
    /// prompt hidden behind it.
    #[test]
    fn an_open_overlay_swallows_keys_it_does_not_act_on() {
        let mut app = App::default();
        app.task_overlay = Some(crate::screens::task_overlay::TaskOverlay::default());

        assert!(handle_task_overlay_key(&mut app, key(KeyCode::Char('q'))));
        assert!(
            app.task_overlay.is_some(),
            "an unhandled key must not close it"
        );
    }

    #[test]
    fn escape_closes_the_overlay() {
        let mut app = App::default();
        app.task_overlay = Some(crate::screens::task_overlay::TaskOverlay::default());

        assert!(handle_task_overlay_key(&mut app, key(KeyCode::Esc)));
        assert!(app.task_overlay.is_none());
    }
}
