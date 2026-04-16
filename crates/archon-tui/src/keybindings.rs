//! Keybindings module for REQ-MOD-020.
//!
//! Centralizes all keyboard shortcut handling into a `KeyMap` with
//! `HashMap<KeyEvent, Action>` and a `resolve()` lookup. The `Action`
//! enum covers every binding found in `main.rs` event loop.

use std::collections::HashMap;

use crossterm::event::{KeyEvent, KeyCode, KeyModifiers};

/// All distinct actions that can be triggered by a key binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Ctrl+C (when not generating) / Ctrl+D = quit the application.
    Quit,
    /// Enter = submit the current input line.
    Submit,
    /// Up arrow = navigate history up or select previous suggestion.
    HistoryUp,
    /// Down arrow = navigate history down or select next suggestion.
    HistoryDown,
    /// PageUp = scroll output up by a page.
    ScrollUp,
    /// PageDown = scroll output down by a page.
    ScrollDown,
    /// Ctrl+Home = scroll to top of output.
    ScrollTop,
    /// Ctrl+End = scroll to bottom of output.
    ScrollBottom,
    /// `/` = open a slash command (the slash is included in the payload).
    SlashCommand(String),
    /// Esc = dismiss suggestions, cancel generation on double-press,
    /// or go back in overlays.
    Escape,
    /// Ctrl+T = toggle thinking display expand/collapse.
    ToggleThinking,
    /// Ctrl+V = voice hotkey trigger.
    VoiceHotkey,
    /// Ctrl+\ = toggle split pane layout.
    ToggleSplit,
    /// Ctrl+W = switch focus between panes.
    SwitchPane,
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
        bindings.insert(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), Action::Quit);
        bindings.insert(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL), Action::Quit);
        bindings.insert(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL), Action::ToggleThinking);
        bindings.insert(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL), Action::VoiceHotkey);
        bindings.insert(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL), Action::ToggleSplit);
        bindings.insert(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL), Action::SwitchPane);

        // Navigation / scroll
        bindings.insert(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), Action::ScrollUp);
        bindings.insert(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), Action::ScrollDown);
        bindings.insert(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL), Action::ScrollTop);
        bindings.insert(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL), Action::ScrollBottom);

        // Core editing keys
        bindings.insert(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), Action::Submit);
        bindings.insert(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), Action::TabComplete);
        bindings.insert(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE), Action::CyclePermissionMode);
        bindings.insert(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), Action::Escape);
        bindings.insert(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), Action::Backspace);
        bindings.insert(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), Action::HistoryUp);
        bindings.insert(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), Action::HistoryDown);
        bindings.insert(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), Action::MoveLeft);
        bindings.insert(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), Action::MoveRight);

        // Slash command
        bindings.insert(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE), Action::SlashCommand("/".to_string()));

        // Character input — NONE and SHIFT modifiers only; Alt/Ctrl are consumed by terminal.
        for &c in ASCII_PRINTABLE {
            bindings.insert(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), Action::CharInput(c));
        }
        for pair in SHIFT_PAIRS {
            let (base, shifted) = *pair;
            bindings.insert(KeyEvent::new(KeyCode::Char(base), KeyModifiers::SHIFT), Action::CharInput(shifted));
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
        self.bindings.get(&key)
    }
}

// ── Character-set constants ───────────────────────────────────────────────────

/// All printable ASCII (0x20–0x7E) except '/' (slash-command trigger).
const ASCII_PRINTABLE: &[char] = &[' ', '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '-', '.', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ':', ';', '<', '=', '>', '?', '@', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '[', '\\', ']', '^', '_', '`', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '{', '|', '}', '~'];

/// (unshifted, shifted) pairs for symbol keys where Shift changes the output.
const SHIFT_PAIRS: &[(char, char)] = &[('1','!'),('2','@'),('3','#'),('4','$'),('5','%'),('6','^'),('7','&'),('8','*'),('9','('),('0',')'),('-','_'),('=','+'),('[','{'),(']','}'),('\\','|'),(';',':'),('\'','"'),(',','<'),('.','>'),('`','~')];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_via_enter() {
        let km = KeyMap::default();
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(km.resolve(enter), Some(&Action::Submit));
    }

    #[test]
    fn quit_via_ctrl_c() {
        let km = KeyMap::default();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(km.resolve(ctrl_c), Some(&Action::Quit));
    }

    #[test]
    fn quit_via_ctrl_d() {
        let km = KeyMap::default();
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(km.resolve(ctrl_d), Some(&Action::Quit));
    }

    #[test]
    fn page_up_scrolls_up() {
        let km = KeyMap::default();
        let pgup = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(km.resolve(pgup), Some(&Action::ScrollUp));
    }

    #[test]
    fn page_down_scrolls_down() {
        let km = KeyMap::default();
        let pgdn = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(km.resolve(pgdn), Some(&Action::ScrollDown));
    }

    #[test]
    fn ctrl_home_scrolls_to_top() {
        let km = KeyMap::default();
        let key = KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL);
        assert_eq!(km.resolve(key), Some(&Action::ScrollTop));
    }

    #[test]
    fn ctrl_end_scrolls_to_bottom() {
        let km = KeyMap::default();
        let key = KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL);
        assert_eq!(km.resolve(key), Some(&Action::ScrollBottom));
    }

    #[test]
    fn escape_key() {
        let km = KeyMap::default();
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(km.resolve(esc), Some(&Action::Escape));
    }

    #[test]
    fn tab_completes() {
        let km = KeyMap::default();
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(km.resolve(tab), Some(&Action::TabComplete));
    }

    #[test]
    fn shift_tab_cycles_permission() {
        let km = KeyMap::default();
        let backtab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
        assert_eq!(km.resolve(backtab), Some(&Action::CyclePermissionMode));
    }

    #[test]
    fn up_arrow_history_up() {
        let km = KeyMap::default();
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(km.resolve(up), Some(&Action::HistoryUp));
    }

    #[test]
    fn down_arrow_history_down() {
        let km = KeyMap::default();
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(km.resolve(down), Some(&Action::HistoryDown));
    }

    #[test]
    fn left_arrow_moves_left() {
        let km = KeyMap::default();
        let left = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(km.resolve(left), Some(&Action::MoveLeft));
    }

    #[test]
    fn right_arrow_moves_right() {
        let km = KeyMap::default();
        let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(km.resolve(right), Some(&Action::MoveRight));
    }

    #[test]
    fn backspace() {
        let km = KeyMap::default();
        let bs = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(km.resolve(bs), Some(&Action::Backspace));
    }

    #[test]
    fn slash_opens_command() {
        let km = KeyMap::default();
        let slash = KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE);
        assert_eq!(km.resolve(slash), Some(&Action::SlashCommand("/".to_string())));
    }

    #[test]
    fn char_input_lowercase() {
        let km = KeyMap::default();
        let a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(km.resolve(a), Some(&Action::CharInput('a')));
    }

    #[test]
    fn char_input_uppercase() {
        let km = KeyMap::default();
        let a = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE);
        assert_eq!(km.resolve(a), Some(&Action::CharInput('A')));
    }

    #[test]
    fn ctrl_t_toggles_thinking() {
        let km = KeyMap::default();
        let key = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(km.resolve(key), Some(&Action::ToggleThinking));
    }

    #[test]
    fn ctrl_v_voice_hotkey() {
        let km = KeyMap::default();
        let key = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert_eq!(km.resolve(key), Some(&Action::VoiceHotkey));
    }

    #[test]
    fn ctrl_backslash_toggles_split() {
        let km = KeyMap::default();
        let key = KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL);
        assert_eq!(km.resolve(key), Some(&Action::ToggleSplit));
    }

    #[test]
    fn ctrl_w_switches_pane() {
        let km = KeyMap::default();
        let key = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(km.resolve(key), Some(&Action::SwitchPane));
    }

    #[test]
    fn unknown_key_returns_none() {
        let km = KeyMap::default();
        let f1 = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(km.resolve(f1), None);
    }

    #[test]
    fn ctrl_a_not_bound() {
        // Ctrl+A is not explicitly bound (it's read by terminal for select-all)
        let km = KeyMap::default();
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert_eq!(km.resolve(key), None);
    }

    #[test]
    fn action_is_char_input() {
        assert!(Action::CharInput('x').is_char_input());
        assert!(!Action::Quit.is_char_input());
        assert!(!Action::Submit.is_char_input());
    }
}
