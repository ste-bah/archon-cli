//! Keybindings module for REQ-MOD-020.
//!
//! Centralizes all keyboard shortcut handling into a `KeyMap` with
//! `HashMap<KeyEvent, Action>` and a `resolve()` lookup. The `Action`
//! enum covers every binding found in `main.rs` event loop.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

/// All distinct actions that can be triggered by a key binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Ctrl+C (when not generating) / Ctrl+D = quit the application.
    Quit,
    /// Enter = submit the current input line.
    Submit,
    /// Shift+Enter / Alt+Enter = insert a newline into the draft instead of
    /// submitting it (issue #174).
    InsertNewline,
    /// Up arrow = navigate history up or select previous suggestion.
    HistoryUp,
    /// Down arrow = navigate history down or select next suggestion.
    HistoryDown,
    /// PageUp / Ctrl+Up = scroll output up by a page.
    ScrollUp,
    /// PageDown / Ctrl+Down = scroll output down by a page.
    ScrollDown,
    /// Ctrl+Home / Ctrl+Left = scroll to top of output.
    ScrollTop,
    /// End / Ctrl+End / Ctrl+Right = scroll to bottom of output.
    ScrollBottom,
    /// `/` = open a slash command (the slash is included in the payload).
    SlashCommand(String),
    /// Esc = dismiss suggestions, cancel generation on double-press,
    /// or go back in overlays.
    Escape,
    /// Ctrl+T = toggle thinking display expand/collapse.
    ToggleThinking,
    /// Ctrl+E = toggle the latest completed tool transcript excerpt.
    ToggleToolOutput,
    /// Ctrl+V = voice hotkey trigger.
    VoiceHotkey,
    /// Ctrl+\ = toggle split pane layout.
    ToggleSplit,
    /// Ctrl+W = switch focus between panes.
    SwitchPane,
    /// Ctrl+O = bring the live activity stream forward.
    OpenActivity,
    /// Ctrl+B = send the live activity stream back to the background.
    BackgroundActivity,
    /// Ctrl+K = open the tasks overlay, where a running task can be cancelled.
    OpenTasks,
    /// Shift+Tab = cycle permission mode.
    CyclePermissionMode,
    /// Tab = accept selected suggestion / autocomplete.
    TabComplete,
    /// Backspace = delete character before cursor.
    Backspace,
    /// Left arrow = move cursor left.
    MoveLeft,
    /// Right arrow = move cursor right.
    MoveRight,
    /// A character key with no modifiers (or Shift only) = insert character.
    CharInput(char),
}

impl Action {
    /// Returns true for actions that represent a printable character insert.
    pub fn is_char_input(&self) -> bool {
        matches!(self, Action::CharInput(_))
    }
}

/// A keymap from `KeyEvent` to `Action`.
#[derive(Debug)]
pub struct KeyMap {
    bindings: HashMap<KeyEvent, Action>,
}

impl Default for KeyMap {
    /// Builds the default keymap matching the bindings in `main.rs`.
    fn default() -> Self {
        let mut bindings = HashMap::new();

        // Control shortcuts
        bindings.insert(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Action::Quit,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Action::Quit,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
            Action::ToggleThinking,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
            Action::ToggleToolOutput,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
            Action::VoiceHotkey,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL),
            Action::ToggleSplit,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
            Action::SwitchPane,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
            Action::OpenActivity,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            Action::BackgroundActivity,
        );
        // Ctrl+K opens the tasks overlay (#189 Phase 9).
        //
        // Ctrl+O's activity stream is keyed by actor role and carries no task
        // id, so it can show that something is running but never stop it. This
        // overlay holds the ids and routes cancellation back through
        // `TaskStore`.
        bindings.insert(
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            Action::OpenTasks,
        );

        // Navigation / scroll
        bindings.insert(
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
            Action::ScrollUp,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL),
            Action::ScrollUp,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            Action::ScrollDown,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL),
            Action::ScrollDown,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL),
            Action::ScrollTop,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
            Action::ScrollTop,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            Action::ScrollBottom,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL),
            Action::ScrollBottom,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
            Action::ScrollBottom,
        );

        // Core editing keys
        bindings.insert(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Action::Submit,
        );
        // Multi-line input (issue #174). BOTH chords are bound, not either:
        //
        // * Shift+Enter is the chord users reach for, but it only ever
        //   arrives with SHIFT set when the terminal is speaking the kitty
        //   keyboard protocol. Without it — Windows Terminal in legacy VT
        //   input, most of xterm's descendants — Shift+Enter is encoded
        //   byte-for-byte as Enter and this binding is simply never hit.
        //   `TerminalGuard::enter` pushes DISAMBIGUATE_ESCAPE_CODES to make
        //   it hit wherever the terminal will allow it.
        // * Alt+Enter survives legacy encoding as an ESC prefix, so it works
        //   everywhere and is the universal fallback. It is documented in
        //   `/help` — a fallback nobody knows about is not a fallback.
        bindings.insert(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
            Action::InsertNewline,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
            Action::InsertNewline,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            Action::TabComplete,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
            Action::CyclePermissionMode,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            Action::CyclePermissionMode,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            Action::Escape,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            Action::Backspace,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            Action::HistoryUp,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            Action::HistoryDown,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            Action::MoveLeft,
        );
        bindings.insert(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            Action::MoveRight,
        );

        // Slash command
        bindings.insert(
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            Action::SlashCommand("/".to_string()),
        );

        // Character input — NONE and SHIFT modifiers only; Alt/Ctrl are consumed by terminal.
        for &c in ASCII_PRINTABLE {
            bindings.insert(
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                Action::CharInput(c),
            );
        }

        // Insert the character the terminal reports, whatever layout produced
        // it. The OS has already resolved the keypress; our job is to accept
        // the answer, not re-derive it.
        //
        // This replaces a hardcoded US table (`('2', '@')`, `('\'', '"')`, …)
        // that mapped the *unshifted* key to a US symbol. On a UK keyboard that
        // table is wrong in both directions: `?` is Shift+/ and was absent
        // entirely, `@` is Shift+' and `~` is Shift+#, so those keys silently
        // did nothing — the event arrived as `Char('?')` WITH shift, and only
        // `Char('?')` with NONE was bound. Confirmed against a real UK terminal:
        // Shift+2 emits `Char('"')` + SHIFT, so the base-key mapping the table
        // assumed never fires there at all.
        //
        // A layout setting in config would be the wrong fix: it needs a table
        // per layout, and breaks for anyone who switches layout mid-session.
        // Letters are deliberately excluded. crossterm normalises `Char('A')`
        // + NONE and `Char('a')` + SHIFT to the SAME hash key, so a blanket
        // identity binding inserted after the NONE loop above silently
        // overwrites `CharInput('A')` with `CharInput('a')` and breaks every
        // capital letter. `char_input_uppercase` catches it; the symbols below
        // have no such collision because their shifted form is a different
        // character entirely, not a case variant.
        for &c in ASCII_PRINTABLE {
            if c.is_ascii_alphabetic() {
                continue;
            }
            bindings.insert(
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT),
                Action::CharInput(c),
            );
        }

        Self { bindings }
    }
}

impl KeyMap {
    /// Look up the action associated with a key event.
    ///
    /// Returns `Some(&Action)` if the event is bound, or `None` if it has
    /// no mapping (e.g. Alt+F4, Ctrl+@, etc.).
    pub fn resolve(&self, key: KeyEvent) -> Option<&Action> {
        self.bindings.get(&normalize(key))
    }
}

/// Reduce a key event to the (code, modifiers) pair the binding table is
/// keyed on.
///
/// `KeyEvent`'s `Hash`/`Eq` include `kind` and `state`, and every binding is
/// built with `KeyEvent::new`, i.e. `Press` + `NONE`. Two real event shapes
/// would otherwise miss every binding:
///
/// * `Repeat` — emitted for held keys (Windows always, and any terminal once
///   `REPORT_EVENT_TYPES` is on). `should_process_key_event` deliberately
///   accepts `Repeat` so held Backspace/arrows work; without this
///   normalisation the accepted event then resolved to nothing.
/// * `KeyEventState::KEYPAD` — set once `DISAMBIGUATE_ESCAPE_CODES` is
///   pushed (issue #174), which is exactly what numeric-keypad Enter carries.
///   Enabling disambiguation without this would have stopped keypad Enter
///   from submitting.
///
/// `Release` never reaches here — `should_process_key_event` drops it first.
fn normalize(key: KeyEvent) -> KeyEvent {
    KeyEvent {
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
        ..key
    }
}

// ── Character-set constants ───────────────────────────────────────────────────

/// All printable ASCII (0x20–0x7E) except '/' (slash-command trigger).
const ASCII_PRINTABLE: &[char] = &[
    ' ', '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '0', '1', '2', '3',
    '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?', '@', 'A', 'B', 'C', 'D', 'E', 'F',
    'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y',
    'Z', '[', '\\', ']', '^', '_', '`', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l',
    'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '{', '|', '}', '~',
];

#[cfg(test)]
#[path = "keybindings_tests.rs"]
mod tests;
