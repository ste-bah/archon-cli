//! App-level key dispatch.
//!
//! Translates a resolved [`Action`] (from [`KeyMap::resolve`]) into calls on
//! [`App`] / [`super::InputHandler`] and returns a [`KeyResult`] describing
//! any side effect (send input, cancel, quit, etc.) for the event loop to
//! execute. Overlay-modal keys (session picker, MCP manager, etc.) are NOT
//! handled here — they stay in `run_tui()` / `event_loop::input`.

use crate::app::App;
use crate::keybindings::{Action, KeyMap};
use crossterm::event::KeyEvent;

/// Result of handling a key event. The caller (run_tui) handles async I/O.
pub enum KeyResult {
    Nothing,
    Quit,
    SendInput(String),
    SendCancel,
    SendBtw(String),
}

/// Process a key event through the KeyMap. Non-modal dispatch only.
/// Overlay-modal keys (session picker, MCP manager, etc.) stay in run_tui().
pub fn handle_key(app: &mut App, key: KeyEvent, keymap: &KeyMap) -> KeyResult {
    let Some(action) = keymap.resolve(key) else {
        return KeyResult::Nothing;
    };

    match action {
        Action::Quit => {
            // Ctrl+C and Ctrl+D both resolve to Quit. Distinguish by raw key.code.
            if matches!(key.code, crossterm::event::KeyCode::Char('c')) && app.is_generating {
                app.is_generating = false;
                app.output.append_line("[interrupted]");
                KeyResult::SendCancel
            } else {
                KeyResult::Quit
            }
        }
        // Grouped single-mutation actions — each calls a no-arg method and returns Nothing
        Action::ToggleThinking => { app.thinking.toggle_expand(); KeyResult::Nothing }
        Action::VoiceHotkey => { crate::voice::pipeline::fire_trigger_for_hotkey(); KeyResult::Nothing }
        Action::ToggleSplit => { app.panes.toggle_split(); KeyResult::Nothing }
        Action::SwitchPane => { app.panes.switch_focus(); KeyResult::Nothing }
        Action::TabComplete => { app.input.accept_suggestion(); KeyResult::Nothing }
        Action::Backspace => { app.input.backspace(); KeyResult::Nothing }
        Action::MoveLeft => { app.input.move_left(); KeyResult::Nothing }
        Action::MoveRight => { app.input.move_right(); KeyResult::Nothing }
        Action::ScrollUp => { app.output.scroll_up(10); KeyResult::Nothing }
        Action::ScrollDown => { app.output.scroll_down(10); KeyResult::Nothing }
        Action::ScrollTop => { app.output.scroll_offset = u16::MAX; app.output.scroll_locked = true; KeyResult::Nothing }
        Action::ScrollBottom => { app.output.scroll_to_bottom(); KeyResult::Nothing }
        Action::Escape => {
            let now = std::time::Instant::now();
            let double_esc = app.last_esc()
                .map(|last| now.duration_since(last).as_millis() < 500)
                .unwrap_or(false);
            if double_esc {
                app.is_generating = false;
                app.active_tool = None;
                app.output.append_line("[interrupted]");
            } else {
                app.input.dismiss_suggestions();
            }
            app.set_last_esc(Some(now));
            KeyResult::Nothing
        }
        Action::CyclePermissionMode => {
            let current = &app.status.permission_mode;
            let modes = ["default", "acceptEdits", "plan", "auto", "dontAsk", "bypassPermissions"];
            let idx = modes.iter().position(|m| m == current).unwrap_or(0);
            let next = modes[(idx + 1) % modes.len()];
            app.status.permission_mode = next.to_string();
            KeyResult::SendInput(format!("/permissions {next}"))
        }
        Action::HistoryUp | Action::HistoryDown => {
            if app.input.suggestions.active {
                if matches!(action, Action::HistoryUp) {
                    app.input.suggestions.select_prev();
                } else {
                    app.input.suggestions.select_next();
                }
            } else if matches!(action, Action::HistoryUp) {
                app.input.history_up();
            } else {
                app.input.history_down();
            }
            KeyResult::Nothing
        }
        Action::CharInput(c) => {
            app.input.insert(*c);
            KeyResult::Nothing
        }
        Action::Submit => {
            // Suggestion popup: exact match → dismiss, else → accept+return
            if app.input.suggestions.active {
                if app.input.suggestions.suggestions.iter().any(|cmd| cmd.name == app.input.text()) {
                    app.input.dismiss_suggestions();
                } else {
                    app.input.accept_suggestion();
                    return KeyResult::Nothing;
                }
            }
            let text = app.submit_input();
            if text.is_empty() { return KeyResult::Nothing; }
            // /btw is always immediate
            if let Some(q) = text.strip_prefix("/btw ").filter(|q| !q.trim().is_empty()) {
                return KeyResult::SendBtw(q.trim().to_string());
            }
            if app.is_generating {
                app.pending_input.push(text);
                app.output.append_line("[queued — will send after current turn]");
                return KeyResult::Nothing;
            }
            KeyResult::SendInput(text)
        }
        Action::SlashCommand(_) => KeyResult::Nothing,
    }
}
