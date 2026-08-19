//! Key routing for the `/model` and `/theme` picker overlays (#192).
//!
//! Split out of `event_loop/input.rs` for the same reason
//! `task_overlay_input.rs` was: that file sits against the 500-line ceiling
//! the `preserve_file_size_ceiling_gate` test enforces, and every new overlay
//! adds another branch to it.
//!
//! Both pickers inject a slash command on Enter rather than applying the
//! change themselves. That is deliberate: `/model` and `/theme` each have one
//! handler that validates and persists, and an overlay that mutated state
//! directly would be a second path with its own bugs.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;

/// Route one key while the model picker is open.
///
/// Returns `true` when the key belonged to the picker, including keys it
/// deliberately swallows — anything falling through would type into the prompt
/// behind the overlay.
pub(crate) fn handle_model_picker_key(app: &mut App, key: KeyEvent) -> bool {
    if app.model_picker.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut picker) = app.model_picker {
                picker.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut picker) = app.model_picker {
                picker.move_down();
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(ref mut picker) = app.model_picker {
                let mut query = picker.query().to_string();
                query.push(c);
                picker.set_query(&query);
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut picker) = app.model_picker {
                let mut query = picker.query().to_string();
                query.pop();
                picker.set_query(&query);
            }
        }
        KeyCode::Enter => {
            if let Some(picker) = app.model_picker.take()
                && let Some(entry) = picker.selected()
            {
                app.input.set_text(&format!("/model {}", entry.model_id));
            }
        }
        KeyCode::Esc => app.model_picker = None,
        _ => {}
    }
    true
}

/// Route one key while the skills menu is open (TASK-TUI-627-followup).
///
/// Moved here alongside the other overlay routing when `input.rs` hit the
/// 500-line ceiling again. Enter injects `/{skill-name} ` and leaves the
/// trailing space, so the user can carry on typing arguments.
pub(crate) fn handle_skills_menu_key(app: &mut App, key: KeyEvent) -> bool {
    if app.skills_menu.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut menu) = app.skills_menu {
                menu.select_prev();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut menu) = app.skills_menu {
                menu.select_next();
            }
        }
        KeyCode::Enter => {
            if let Some(menu) = app.skills_menu.take()
                && let Some(skill) = menu.selected()
            {
                app.input.set_text(&format!("/{} ", skill.name));
            }
        }
        KeyCode::Esc => app.skills_menu = None,
        _ => {}
    }
    true
}

/// Route one key while the theme picker is open.
pub(crate) fn handle_theme_picker_key(app: &mut App, key: KeyEvent) -> bool {
    if app.theme_screen.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut screen) = app.theme_screen {
                screen.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut screen) = app.theme_screen {
                screen.move_down();
            }
        }
        KeyCode::Enter => {
            if let Some(screen) = app.theme_screen.take()
                && let Some(entry) = screen.selected()
            {
                app.input.set_text(&format!("/theme {}", entry.name));
            }
        }
        KeyCode::Esc => app.theme_screen = None,
        _ => {}
    }
    true
}

/// Route one key while the settings overlay is open.
///
/// Enter injects `/config <key> <value>` rather than writing the value here:
/// that command validates the type and refuses a read-only key, and a second
/// path that skipped it would be a second set of bugs. A read-only row still
/// injects, so the refusal comes from the one place that knows why.
pub(crate) fn handle_settings_key(app: &mut App, key: KeyEvent) -> bool {
    if app.settings_screen.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut screen) = app.settings_screen {
                screen.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut screen) = app.settings_screen {
                screen.move_down();
            }
        }
        KeyCode::PageUp => {
            if let Some(ref mut screen) = app.settings_screen {
                screen.page_up();
            }
        }
        KeyCode::PageDown => {
            if let Some(ref mut screen) = app.settings_screen {
                screen.page_down();
            }
        }
        KeyCode::Enter => {
            if let Some(screen) = app.settings_screen.take()
                && let Some(field) = screen.selected()
            {
                app.input.set_text(&field.command());
            }
        }
        KeyCode::Esc => app.settings_screen = None,
        _ => {}
    }
    true
}

/// Route one key while the hooks overlay is open.
///
/// Enter injects `/hooks enable <id>` or `/hooks disable <id>`. That is the
/// command that writes `.archon/hooks.local.toml`; setting the flag here
/// would change the list and nothing else.
pub(crate) fn handle_hooks_key(app: &mut App, key: KeyEvent) -> bool {
    if app.hooks_menu.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut menu) = app.hooks_menu {
                menu.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut menu) = app.hooks_menu {
                menu.move_down();
            }
        }
        KeyCode::PageUp => {
            if let Some(ref mut menu) = app.hooks_menu {
                menu.page_up();
            }
        }
        KeyCode::PageDown => {
            if let Some(ref mut menu) = app.hooks_menu {
                menu.page_down();
            }
        }
        KeyCode::Enter => {
            if let Some(menu) = app.hooks_menu.take()
                && let Some(hook) = menu.selected()
            {
                app.input.set_text(&hook.command());
            }
        }
        KeyCode::Esc => app.hooks_menu = None,
        _ => {}
    }
    true
}

/// Route one key while the permission-rules overlay is open.
///
/// There is no Enter action. These rules are read once from `[permissions]` at
/// session start and nothing at runtime can change them, so the overlay reads
/// and closes. Offering an Enter that did nothing would be worse than not
/// offering one.
pub(crate) fn handle_permissions_key(app: &mut App, key: KeyEvent) -> bool {
    if app.permissions_browser.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut browser) = app.permissions_browser {
                browser.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut browser) = app.permissions_browser {
                browser.move_down();
            }
        }
        KeyCode::PageUp => {
            if let Some(ref mut browser) = app.permissions_browser {
                browser.page_up();
            }
        }
        KeyCode::PageDown => {
            if let Some(ref mut browser) = app.permissions_browser {
                browser.page_down();
            }
        }
        KeyCode::Esc | KeyCode::Enter => app.permissions_browser = None,
        _ => {}
    }
    true
}

/// Route one key while the memory-files overlay is open.
///
/// Typing filters by path. There is no Enter action — the files are read into
/// the system prompt at startup and the TUI has no editor to hand one to.
pub(crate) fn handle_memory_files_key(app: &mut App, key: KeyEvent) -> bool {
    if app.memory_browser.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut browser) = app.memory_browser {
                browser.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut browser) = app.memory_browser {
                browser.move_down();
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(ref mut browser) = app.memory_browser {
                let mut query = browser.query().to_string();
                query.push(c);
                browser.set_query(&query);
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut browser) = app.memory_browser {
                let mut query = browser.query().to_string();
                query.pop();
                browser.set_query(&query);
            }
        }
        KeyCode::Esc | KeyCode::Enter => app.memory_browser = None,
        _ => {}
    }
    true
}

/// Route one key while the branch picker is open.
///
/// Enter injects `/branch <index>`. The command owns the fork, so there is one
/// path that decides what a branch means rather than two.
pub(crate) fn handle_branch_picker_key(app: &mut App, key: KeyEvent) -> bool {
    if app.branch_picker.is_none() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            if let Some(ref mut picker) = app.branch_picker {
                picker.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut picker) = app.branch_picker {
                picker.move_down();
            }
        }
        KeyCode::PageUp => {
            if let Some(ref mut picker) = app.branch_picker {
                picker.page_up();
            }
        }
        KeyCode::PageDown => {
            if let Some(ref mut picker) = app.branch_picker {
                picker.page_down();
            }
        }
        KeyCode::Enter => {
            if let Some(picker) = app.branch_picker.take()
                && let Some(entry) = picker.selected()
            {
                app.input.set_text(&entry.command());
            }
        }
        KeyCode::Esc => app.branch_picker = None,
        _ => {}
    }
    true
}

/// Route one key while the voice capture overlay is open.
///
/// Esc closes it, and cancels the recording if one is running — closing the
/// window on a live microphone and leaving it recording is the behaviour
/// nobody wants. Enter closes without cancelling, so a finished recording can
/// be dismissed while its transcription is already in the input line.
pub(crate) fn handle_voice_capture_key(app: &mut App, key: KeyEvent) -> bool {
    let Some(overlay) = app.voice_capture.as_ref() else {
        return false;
    };
    match key.code {
        KeyCode::Esc => {
            if overlay.is_recording() {
                // Best effort: with no voice pipeline running this is a no-op,
                // and the overlay still closes.
                let _ = crate::voice::pipeline::fire_trigger(
                    crate::voice::pipeline::VoiceTrigger::Cancel,
                );
            }
            app.voice_capture = None;
        }
        KeyCode::Enter => app.voice_capture = None,
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn keys_pass_through_when_no_picker_is_open() {
        let mut app = App::default();
        assert!(!handle_model_picker_key(&mut app, key(KeyCode::Down)));
        assert!(!handle_theme_picker_key(&mut app, key(KeyCode::Down)));
    }

    /// Every key is consumed while a picker is up, or typing lands in the
    /// prompt hidden behind it.
    #[test]
    fn every_key_is_consumed_while_the_model_picker_is_open() {
        let mut app = App::default();
        app.model_picker = Some(crate::screens::model_picker::ModelPicker::new());
        for code in [KeyCode::Tab, KeyCode::Home, KeyCode::PageUp, KeyCode::F(5)] {
            assert!(
                handle_model_picker_key(&mut app, key(code)),
                "{code:?} fell through to the input behind the overlay"
            );
        }
    }
}
