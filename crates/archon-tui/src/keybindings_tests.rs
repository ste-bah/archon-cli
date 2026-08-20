//! Keymap resolution tests, split out to keep `keybindings.rs` under the
//! 500-line file-size gate.

use super::*;

#[test]
fn submit_via_enter() {
    let km = KeyMap::default();
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(km.resolve(enter), Some(&Action::Submit));
}

/// Issue #174 acceptance: Enter submits, Shift+Enter and Alt+Enter insert a
/// newline. Alt is bound as well as Shift because Shift+Enter only reaches us
/// with SHIFT set when the terminal speaks the kitty keyboard protocol.
#[test]
fn shift_enter_and_alt_enter_insert_a_newline() {
    let km = KeyMap::default();
    assert_eq!(
        km.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
        Some(&Action::InsertNewline)
    );
    assert_eq!(
        km.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        Some(&Action::InsertNewline)
    );
    assert_eq!(
        km.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Some(&Action::Submit),
        "plain Enter must still submit"
    );
}

/// `DISAMBIGUATE_ESCAPE_CODES` makes the numeric keypad's Enter arrive with
/// `KeyEventState::KEYPAD`. Without normalisation that key would stop
/// submitting the moment the enhancement flags were pushed.
#[test]
fn keypad_enter_still_submits_under_disambiguation() {
    let km = KeyMap::default();
    let keypad_enter = KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::KEYPAD,
    };
    assert_eq!(km.resolve(keypad_enter), Some(&Action::Submit));
}

/// Held keys arrive as `Repeat`; `should_process_key_event` accepts them, so
/// resolution must too.
#[test]
fn repeat_events_resolve_like_press_events() {
    let km = KeyMap::default();
    let held_backspace = KeyEvent {
        code: KeyCode::Backspace,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Repeat,
        state: KeyEventState::NONE,
    };
    assert_eq!(km.resolve(held_backspace), Some(&Action::Backspace));
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
fn ctrl_up_scrolls_up_for_wsl_terminals() {
    let km = KeyMap::default();
    let key = KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL);
    assert_eq!(km.resolve(key), Some(&Action::ScrollUp));
}

#[test]
fn page_down_scrolls_down() {
    let km = KeyMap::default();
    let pgdn = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
    assert_eq!(km.resolve(pgdn), Some(&Action::ScrollDown));
}

#[test]
fn ctrl_down_scrolls_down_for_wsl_terminals() {
    let km = KeyMap::default();
    let key = KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL);
    assert_eq!(km.resolve(key), Some(&Action::ScrollDown));
}

#[test]
fn ctrl_home_scrolls_to_top() {
    let km = KeyMap::default();
    let key = KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL);
    assert_eq!(km.resolve(key), Some(&Action::ScrollTop));
}

#[test]
fn ctrl_left_scrolls_to_top_for_wsl_terminals() {
    let km = KeyMap::default();
    let key = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
    assert_eq!(km.resolve(key), Some(&Action::ScrollTop));
}

#[test]
fn ctrl_right_scrolls_to_bottom_for_wsl_terminals() {
    let km = KeyMap::default();
    let key = KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL);
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
fn shift_tab_with_shift_modifier_cycles_permission() {
    let km = KeyMap::default();
    let backtab_shift = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(
        km.resolve(backtab_shift),
        Some(&Action::CyclePermissionMode)
    );
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
    assert_eq!(
        km.resolve(slash),
        Some(&Action::SlashCommand("/".to_string()))
    );
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
fn ctrl_o_opens_activity_stream() {
    let km = KeyMap::default();
    let key = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(km.resolve(key), Some(&Action::OpenActivity));
}

#[test]
fn ctrl_b_backgrounds_activity_stream() {
    let km = KeyMap::default();
    let key = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(km.resolve(key), Some(&Action::BackgroundActivity));
}

#[test]
fn ctrl_k_opens_the_tasks_overlay() {
    let km = KeyMap::default();
    let key = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert_eq!(km.resolve(key), Some(&Action::OpenTasks));
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

/// Non-US layouts must be able to type their symbols.
///
/// This replaced a hardcoded US pair table under which `?` (Shift+/ on a UK
/// keyboard) was absent entirely and `@` / `~` sat on different keys than
/// it assumed, so those keys silently did nothing. Terminals resolve the
/// layout and report the resulting character with SHIFT set, so each
/// printable character is bound to itself under SHIFT.
#[test]
fn shifted_symbols_resolve_on_non_us_layouts() {
    let km = KeyMap::default();
    for c in ['?', '@', '~', '"', ':', '_', '+'] {
        let shifted = KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);
        assert_eq!(
            km.resolve(shifted),
            Some(&Action::CharInput(c)),
            "shifted {c:?} must insert {c:?}"
        );
    }
}

/// Ctrl+P toggles reading replies aloud (#192 follow-up).
///
/// Not Ctrl+S: that is XOFF on a terminal, and binding speech to it would
/// freeze output on any machine where flow control is still enabled.
#[test]
fn ctrl_p_toggles_speech() {
    let km = KeyMap::default();
    assert_eq!(
        km.resolve(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        Some(&Action::ToggleSpeech)
    );
    assert_ne!(
        km.resolve(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        Some(&Action::ToggleSpeech),
        "Ctrl+S is XOFF and must not be the speech key"
    );
}
