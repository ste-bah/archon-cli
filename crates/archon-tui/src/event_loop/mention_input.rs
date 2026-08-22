//! Trigger and key routing for the `@`-mention picker (#200 Phase 4).
//!
//! Split out of `picker_input.rs`, which already sits within thirty lines of
//! the 500-line ceiling, and which is the wrong shape for this anyway: every
//! handler in there swallows all keys while its overlay is up. This one must
//! not. The mention picker is a *completion*, so ordinary characters and
//! Backspace have to keep reaching the prompt underneath, or the user could
//! not narrow the list by typing — the one thing the list is for.
//!
//! The picker's whole lifecycle is derived from the buffer by
//! [`sync_session_mention`], which runs after every edit. There is no separate
//! "mention mode" flag that could survive an edit that ended the mention, and
//! nothing has to remember to close the overlay: if the caret is no longer
//! inside a mention, the scan says so and the picker goes away.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::screens::session_mention::SessionMentionPicker;

/// Open, update or close the picker to match what the caret is inside.
///
/// Candidates are read once, when the mention opens, rather than on every
/// keystroke: the list is only there to name an id, and the reference itself
/// is resolved against the store at send time (see the bin crate's
/// `session_loop::mention_resolve`). A row going stale between `@` and Enter
/// therefore cannot produce a stale snapshot — at worst it names a session
/// that no longer resolves, and that is reported as an error.
pub(crate) fn sync_session_mention(app: &mut App) {
    let Some(mention) =
        archon_core::mention::active_at_cursor(app.input.text(), app.input.cursor())
    else {
        app.session_mention = None;
        return;
    };
    if app.session_mention.is_none() {
        app.session_mention = Some(match app.session_mention_source.as_ref() {
            Some(source) => SessionMentionPicker::new(source.candidates()),
            None => SessionMentionPicker::unavailable(),
        });
    }
    if let Some(picker) = app.session_mention.as_mut() {
        picker.set_query(&mention.query);
    }
}

/// Route one key while the mention picker is open.
///
/// Returns `true` only for the keys the picker owns. Everything else — text,
/// Backspace, cursor movement — returns `false` and reaches the prompt, after
/// which [`sync_session_mention`] re-derives the picker from the new buffer.
pub(crate) fn handle_session_mention_key(app: &mut App, key: KeyEvent) -> bool {
    if app.session_mention.is_none() || key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    match key.code {
        KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
            if let Some(picker) = app.session_mention.as_mut() {
                match key.code {
                    KeyCode::Up => picker.move_up(),
                    KeyCode::Down => picker.move_down(),
                    KeyCode::PageUp => picker.page_up(),
                    _ => picker.page_down(),
                }
            }
            true
        }
        // Esc dismisses the list and leaves the typed text exactly as it is.
        // Re-deriving the picker from the buffer would reopen it instantly, so
        // the dismissal has to outlast this key; the next edit is what may
        // bring it back, which is what "I meant a literal @" needs.
        KeyCode::Esc => {
            app.session_mention = None;
            true
        }
        KeyCode::Enter | KeyCode::Tab => {
            accept_session_mention(app);
            true
        }
        _ => false,
    }
}

/// Write the highlighted session into the buffer, in place.
///
/// Enter with nothing highlighted — an empty or fully filtered list — closes
/// the picker and does nothing else. It deliberately does not fall through to
/// sending the turn: Enter arriving while an overlay is up should never be the
/// keystroke that dispatches a message the user cannot see the whole of.
fn accept_session_mention(app: &mut App) {
    let Some(picker) = app.session_mention.take() else {
        return;
    };
    let Some(chosen) = picker.selected() else {
        return;
    };
    let Some(mention) =
        archon_core::mention::active_at_cursor(app.input.text(), app.input.cursor())
    else {
        // The buffer stopped containing a mention between the scan that
        // opened the picker and this keypress. Writing the token in blind
        // would corrupt whatever is there now.
        tracing::debug!("mention picker accepted with no active mention; nothing written");
        return;
    };
    let (text, cursor) =
        archon_core::mention::replace_active(app.input.text(), &mention, &chosen.id);
    app.input.set_text_with_cursor(&text, cursor);
}

#[cfg(test)]
#[path = "mention_input_tests.rs"]
mod tests;
