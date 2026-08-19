//! App-level key dispatch.
//!
//! Translates a resolved [`Action`] (from [`KeyMap::resolve`]) into calls on
//! [`App`] / [`super::InputHandler`] and returns a [`KeyResult`] describing
//! any side effect (send input, cancel, quit, etc.) for the event loop to
//! execute. Overlay-modal keys (session picker, MCP manager, etc.) remain in
//! `event_loop::input`, while `session_loop` consumes async send/cancel results.

use crate::app::App;
use crate::keybindings::{Action, KeyMap};
use crossterm::event::{KeyEvent, KeyModifiers};

/// Result of handling a key event. `session_loop` handles async I/O.
pub enum KeyResult {
    Nothing,
    Quit,
    SendInput(String),
    SendCancel,
    SendBtw(String),
}

/// Process a key event through the KeyMap. Non-modal dispatch only.
/// Overlay-modal keys (session picker, MCP manager, etc.) stay in
/// `event_loop::input`.
pub fn handle_key(app: &mut App, key: KeyEvent, keymap: &KeyMap) -> KeyResult {
    let resolved = keymap.resolve(key).cloned();
    let action = match resolved {
        Some(action) => action,
        // Any printable character the keymap does not enumerate still types.
        //
        // The keymap is an explicit ASCII list, so a UK keyboard's `£` and `¬`
        // -- which crossterm delivers verbatim as `Char('£')` / `Char('¬')` --
        // resolved to nothing and the key silently did nothing. Extending the
        // list would only move the boundary: it cannot enumerate every layout's
        // characters, and each omission fails the same silent way.
        //
        // CONTROL and ALT are excluded so an unbound chord (Ctrl+A, Alt+F4)
        // keeps doing nothing rather than typing a stray letter; SHIFT alone is
        // just a shifted character. Control characters are excluded because
        // Enter, Tab and Backspace arrive as their own `KeyCode`s, not as
        // `Char`, so anything reaching here as a control char is not text.
        None => match key.code {
            crossterm::event::KeyCode::Char(c)
                if !c.is_control()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                Action::CharInput(c)
            }
            _ => return KeyResult::Nothing,
        },
    };
    let action = &action;

    if app.activity_stream.is_foreground() {
        return handle_activity_key(app, action);
    }

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
        Action::ToggleThinking => {
            app.toggle_thinking();
            KeyResult::Nothing
        }
        Action::ToggleToolOutput => {
            app.toggle_tool_output(None);
            KeyResult::Nothing
        }
        Action::VoiceHotkey => {
            crate::voice::pipeline::fire_trigger_for_hotkey();
            KeyResult::Nothing
        }
        Action::ToggleSplit => {
            app.panes.toggle_split();
            KeyResult::Nothing
        }
        Action::SwitchPane => {
            app.panes.switch_focus();
            KeyResult::Nothing
        }
        Action::OpenActivity => {
            app.open_activity_stream();
            KeyResult::Nothing
        }
        Action::BackgroundActivity => {
            app.background_activity_stream();
            KeyResult::Nothing
        }
        Action::OpenTasks => {
            app.toggle_task_overlay();
            KeyResult::Nothing
        }
        Action::TabComplete => {
            app.input.accept_suggestion();
            KeyResult::Nothing
        }
        Action::Backspace => {
            app.input.backspace();
            KeyResult::Nothing
        }
        Action::MoveLeft => {
            app.input.move_left();
            KeyResult::Nothing
        }
        Action::MoveRight => {
            app.input.move_right();
            KeyResult::Nothing
        }
        Action::ScrollUp if app.thinking.active && app.thinking.expanded => {
            app.thinking.scroll_up(10);
            KeyResult::Nothing
        }
        Action::ScrollDown if app.thinking.active && app.thinking.expanded => {
            app.thinking.scroll_down(10);
            KeyResult::Nothing
        }
        Action::ScrollTop if app.thinking.active && app.thinking.expanded => {
            app.thinking.scroll_to_top();
            KeyResult::Nothing
        }
        Action::ScrollBottom if app.thinking.active && app.thinking.expanded => {
            app.thinking.scroll_to_bottom();
            KeyResult::Nothing
        }
        Action::ScrollUp => {
            app.output.scroll_up(10);
            KeyResult::Nothing
        }
        Action::ScrollDown => {
            app.output.scroll_down(10);
            KeyResult::Nothing
        }
        Action::ScrollTop => {
            app.output.scroll_to_top();
            KeyResult::Nothing
        }
        Action::ScrollBottom => {
            app.output.scroll_to_bottom();
            KeyResult::Nothing
        }
        Action::Escape => {
            let now = std::time::Instant::now();
            let double_esc = app
                .last_esc()
                .map(|last| now.duration_since(last).as_millis() < 500)
                .unwrap_or(false);
            app.set_last_esc(Some(now));
            // Bug-fix 2026-05-12: double-Esc must emit SendCancel so the cancel
            // chain (__cancel__ → AgentHandle::fire_cancel → JoinHandle::abort
            // → CancellationToken cascade) actually fires. Previously this
            // path only flipped display flags and printed "[interrupted]" —
            // theater with no real effect on the in-flight turn.
            if double_esc && app.is_generating {
                app.is_generating = false;
                app.active_tool = None;
                app.output.append_line("[interrupted]");
                KeyResult::SendCancel
            } else {
                if !double_esc {
                    app.input.dismiss_suggestions();
                }
                KeyResult::Nothing
            }
        }
        Action::CyclePermissionMode => {
            let current = &app.status.permission_mode;
            let modes = [
                "default",
                "acceptEdits",
                "plan",
                "auto",
                "dontAsk",
                "bypassPermissions",
            ];
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
        // Shift+Enter / Alt+Enter (issue #174). Deliberately *before* the
        // suggestion handling that `Submit` does: growing a draft onto a
        // second line is never an attempt to accept a slash completion.
        Action::InsertNewline => {
            app.input.insert_newline();
            KeyResult::Nothing
        }
        Action::Submit => {
            // Suggestion popup: exact match → dismiss, else → accept+return
            if app.input.suggestions.active {
                if app
                    .input
                    .suggestions
                    .suggestions
                    .iter()
                    .any(|cmd| cmd.name == app.input.text())
                {
                    app.input.dismiss_suggestions();
                } else {
                    app.input.accept_suggestion();
                    return KeyResult::Nothing;
                }
            }
            let text = app.submit_input();
            if text.is_empty() {
                return KeyResult::Nothing;
            }
            // /btw is always immediate
            if let Some(q) = text.strip_prefix("/btw ").filter(|q| !q.trim().is_empty()) {
                return KeyResult::SendBtw(q.trim().to_string());
            }
            if text == "/thinking" || text.starts_with("/thinking ") {
                return KeyResult::SendInput(text);
            }
            if app.is_generating {
                app.pending_input.push(text);
                app.output
                    .append_line("[queued — will send after current turn]");
                return KeyResult::Nothing;
            }
            KeyResult::SendInput(text)
        }
        Action::SlashCommand(_) => {
            app.input.insert('/');
            KeyResult::Nothing
        }
    }
}

fn handle_activity_key(app: &mut App, action: &Action) -> KeyResult {
    match action {
        Action::BackgroundActivity | Action::Escape => app.background_activity_stream(),
        Action::OpenActivity => app.open_activity_stream(),
        Action::ScrollUp => app.activity_stream.scroll_up(),
        Action::ScrollDown => app.activity_stream.scroll_down(),
        Action::ScrollTop => app.activity_stream.scroll_top(),
        Action::ScrollBottom => app.activity_stream.scroll_bottom(),
        Action::HistoryUp => app.activity_stream.select_prev(),
        Action::HistoryDown => app.activity_stream.select_next(),
        _ => {}
    }
    KeyResult::Nothing
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    #[test]
    fn thinking_command_bypasses_generation_queue() {
        let mut app = App::new();
        app.is_generating = true;
        app.input.inject_text("/thinking on");
        let keymap = KeyMap::default();

        let result = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &keymap,
        );

        assert!(matches!(result, KeyResult::SendInput(ref text) if text == "/thinking on"));
        assert!(app.pending_input.is_empty());
        assert!(app.is_generating);
        assert!(
            !app.output
                .all_lines()
                .contains(&"[queued — will send after current turn]")
        );
    }

    #[test]
    fn other_slash_commands_remain_queued_during_generation() {
        let mut app = App::new();
        app.is_generating = true;
        app.input.inject_text("/help");
        let keymap = KeyMap::default();

        let result = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &keymap,
        );

        assert!(matches!(result, KeyResult::Nothing));
        assert_eq!(app.pending_input, ["/help"]);
    }

    #[test]
    fn end_restores_transcript_follow_after_streamed_arrivals() {
        let mut app = App::new();
        for index in 0..30 {
            app.output.append_line(&format!("existing-{index}"));
        }
        let theme = crate::theme::intj_theme();
        app.output.rendered_view(&theme, 20, 10);
        app.output.scroll_up(10);
        app.output.append_line("streamed-arrival");
        assert!(app.output.scroll_locked);

        let keymap = KeyMap::default();
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            &keymap,
        );
        assert!(!app.output.scroll_locked);
        assert_eq!(app.output.scroll_offset, 0);

        app.output.append_line("later-arrival");
        let view = app.output.rendered_view(&theme, 20, 10);
        assert_eq!(view.global_scroll_y, view.total_wrapped.saturating_sub(10));
    }

    #[test]
    fn expanded_active_thinking_scrolls_without_moving_transcript() {
        let mut app = App::new();
        app.thinking.active = true;
        app.thinking.expanded = true;
        let keymap = KeyMap::default();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
            &keymap,
        );

        assert_eq!(app.thinking.scroll_offset, 10);
        assert_eq!(app.output.scroll_offset, 0);
        assert!(!app.output.scroll_locked);
    }
}
